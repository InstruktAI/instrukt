//! Review evidence for exact commits no todo's record carries
//! (`project/instrukt/spec/feature/work/github-apps`, Review evidence,
//! Admission of review evidence, Review evidence storage): head verdicts
//! with their versions, exemptions, and the answer runner and workflow read
//! on a commit.

mod _admission;
mod _moment;
mod _publication;
mod _published;
mod _recognition;
mod _store;
mod _transfer;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use instrukt_core::git;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef};
use instrukt_runner::state::RunnerState;
use jiff::Timestamp;

pub use _moment::Moment;
pub use _publication::Published;
pub(crate) use _publication::shortened;
pub use _published::{Delivery, DeliveryState, Derived, Destination, Recognised};
pub use _transfer::Statement;

use _admission::{Offer, RequestCheck, TurnDecision};
use _store::Held;

use crate::evidence::ContentDigest;

/// The remote whose canonical branch evidence is judged against.
const REMOTE: &str = "origin";

/// How long a recording command asks a runtime that cannot say again.
const TURN_WAIT: Duration = Duration::from_secs(30);

/// How often within that wait the runtime is asked again.
const TURN_RETRY: Duration = Duration::from_secs(1);

/// A review's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    Approve,
    NeedsWork,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Approve => "approve",
            Verdict::NeedsWork => "needs_work",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "approve" => Some(Verdict::Approve),
            "needs_work" => Some(Verdict::NeedsWork),
            _ => None,
        }
    }
}

/// Where a head verdict is on this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// It is the commit's verdict here.
    Stands,
    /// An export handed it off.
    HandedOff,
    /// An import set it aside for another.
    SetAside,
}

impl Standing {
    pub fn as_str(self) -> &'static str {
        match self {
            Standing::Stands => "stands",
            Standing::HandedOff => "handed_off",
            Standing::SetAside => "set_aside",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "stands" => Some(Standing::Stands),
            "handed_off" => Some(Standing::HandedOff),
            "set_aside" => Some(Standing::SetAside),
            _ => None,
        }
    }
}

/// One version of a head verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub number: u64,
    pub verdict: Verdict,
    /// The SHA-256 of the report as supplied: the version's name.
    pub name: ContentDigest,
    pub report: String,
    pub reviewer: SessionRef,
    /// The session that entered a handed-over approval.
    pub entered_by: Option<SessionRef>,
    /// The pinned base the review started from.
    pub base: CommitSha,
    pub base_on_canonical: bool,
    /// The review request it answers.
    pub request: Option<u64>,
    /// The reviewed commits that name no authoring session.
    pub unattributed: Vec<CommitSha>,
    pub moment: Moment,
    pub recorded_at: Timestamp,
    /// The computer it was recorded on.
    pub recorded_on: String,
}

/// One head verdict of a commit with its versions, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    pub standing: Standing,
    /// What handed it off or set it aside.
    pub cause: Option<String>,
    pub changed_at: Timestamp,
    pub versions: Vec<Version>,
}

/// A commit's exemption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exemption {
    pub commit: CommitSha,
    pub basis: String,
    pub declared_by: Option<SessionRef>,
    /// Whether the commit names an authoring session.
    pub names_session: bool,
    pub recorded_at: Timestamp,
    pub recorded_on: String,
}

/// Everything this computer holds of a commit's review evidence, with what
/// the tick published of it and the verdict it recognised as already
/// published.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitEvidence {
    pub settlements: Vec<Settlement>,
    pub exemption: Option<Exemption>,
    /// Its derived approval: standing, or withdrawn with the commit found
    /// uncovered.
    pub derived: Option<Derived>,
    pub deliveries: Vec<Delivery>,
    pub recognised: Option<Recognised>,
}

impl CommitEvidence {
    /// Whether the commit carries a verdict here: a head verdict that
    /// stands, a derived approval that was not withdrawn, or a verdict
    /// recognised as already published (`github-apps`, Review evidence).
    pub fn carries_verdict(&self) -> bool {
        self.current().is_some()
            || self.derived.as_ref().is_some_and(|d| d.withdrawn.is_none())
            || self.recognised.is_some()
    }

    /// The current version of the head verdict that stands here.
    pub fn current(&self) -> Option<&Version> {
        self.settlements
            .iter()
            .find(|settlement| settlement.standing == Standing::Stands)
            .and_then(|settlement| settlement.versions.last())
    }
}

/// A review command was refused or could not complete.
#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error("{0}")]
    Refused(String),
    #[error("the review evidence store holds an unreadable {0}")]
    Record(String),
    #[error(transparent)]
    Store(#[from] instrukt_core::state_db::StoreError),
    #[error(transparent)]
    Sql(#[from] instrukt_core::state_db::rusqlite::Error),
    #[error(transparent)]
    Git(#[from] git::GitError),
    #[error(transparent)]
    Runner(#[from] instrukt_runner::review::ReviewReadError),
    #[error(transparent)]
    Machine(#[from] instrukt_core::machine::MachineError),
}

fn refused(message: impl Into<String>) -> ReviewError {
    ReviewError::Refused(message.into())
}

/// A project's repository as review evidence reads it.
#[derive(Debug, Clone)]
pub struct ReviewProject {
    pub slug: ProjectSlug,
    pub root: PathBuf,
}

impl ReviewProject {
    /// The full id of commit `text` in this repository; refused otherwise.
    fn commit(&self, text: &str) -> Result<CommitSha, ReviewError> {
        let not_here = || {
            refused(format!(
                "`{text}` is not the full id of a commit in project {}'s repository; give the 40-character commit id",
                self.slug
            ))
        };
        let sha = CommitSha::parse(text).map_err(|_| not_here())?;
        // `rev-parse --verify --quiet` exits 1 for an id the repository does
        // not hold or that names no commit; `cat-file -e` exits 128 there.
        let object = format!("{sha}^{{commit}}");
        if !git::succeeds(
            &self.root,
            ["rev-parse", "--verify", "--quiet", object.as_str()],
        )? {
            return Err(not_here());
        }
        Ok(sha)
    }

    /// The canonical branch as this checkout knows it: the remote-tracking
    /// branch when it exists, else the local one.
    fn canonical(&self) -> Result<String, ReviewError> {
        let branch = git::default_branch(&self.root, REMOTE)?;
        let remote = format!("refs/remotes/{REMOTE}/{branch}");
        Ok(
            if git::succeeds(
                &self.root,
                ["rev-parse", "--verify", "--quiet", remote.as_str()],
            )? {
                remote
            } else {
                branch
            },
        )
    }

    /// Whether this repository holds `commit`: evidence is kept for commits
    /// it does not hold, as an import may bring them or `git gc` may prune
    /// an unmerged one.
    fn holds(&self, commit: &CommitSha) -> Result<bool, ReviewError> {
        Ok(git::succeeds(
            &self.root,
            ["cat-file", "-e", commit.as_str()],
        )?)
    }

    /// Whether a branch of the project, local or fetched from the remote,
    /// still reaches `commit`.
    fn reached(&self, commit: &CommitSha) -> Result<bool, ReviewError> {
        let remote = format!("refs/remotes/{REMOTE}");
        Ok(!git::run(
            &self.root,
            [
                "for-each-ref",
                "--count=1",
                "--format=%(refname)",
                "--contains",
                commit.as_str(),
                "refs/heads",
                remote.as_str(),
            ],
        )?
        .trim()
        .is_empty())
    }

    fn contains(&self, ancestor: &str, descendant: &str) -> Result<bool, ReviewError> {
        Ok(git::succeeds(
            &self.root,
            ["merge-base", "--is-ancestor", ancestor, descendant],
        )?)
    }

    fn merge_base(&self, a: &str, b: &str) -> Result<Option<CommitSha>, ReviewError> {
        let output = git::run_output(&self.root, ["merge-base", a, b])?;
        Ok(output
            .status
            .success()
            .then(|| CommitSha::parse(String::from_utf8_lossy(&output.stdout).trim()).ok())
            .flatten())
    }

    /// Each commit after `base` up to `head` with the sessions its
    /// `Session:` trailers name.
    fn sessions_in(
        &self,
        base: &CommitSha,
        head: &CommitSha,
    ) -> Result<Vec<(CommitSha, Vec<String>)>, ReviewError> {
        self.trailers(&[&format!("{base}..{head}")])
    }

    /// The sessions `commit`'s `Session:` trailers name.
    fn sessions_of(&self, commit: &CommitSha) -> Result<Vec<String>, ReviewError> {
        Ok(self
            .trailers(&["-1", commit.as_str()])?
            .into_iter()
            .next()
            .map(|(_, sessions)| sessions)
            .unwrap_or_default())
    }

    /// The commits `revisions` select, each with the sessions its
    /// `Session:` trailers name.
    fn trailers(&self, revisions: &[&str]) -> Result<Vec<(CommitSha, Vec<String>)>, ReviewError> {
        let mut args = vec![
            "log",
            "--format=%H%x1f%(trailers:key=Session,valueonly,separator=%x1e)%x1d",
        ];
        args.extend(revisions);
        let text = git::run(&self.root, args)?;
        text.split('\u{1d}')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                let (sha, sessions) = entry.split_once('\u{1f}').unwrap_or((entry, ""));
                let sha = CommitSha::parse(sha.trim())
                    .map_err(|_| ReviewError::Record(format!("commit id `{sha}` from git")))?;
                let sessions = sessions
                    .split('\u{1e}')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
                Ok((sha, sessions))
            })
            .collect()
    }
}

/// A head verdict offered for recording.
#[derive(Debug, Clone)]
pub struct RecordRequest {
    pub head: String,
    pub base: String,
    pub verdict: Verdict,
    pub report: String,
    /// The calling session, when one resolves.
    pub caller: Option<SessionRef>,
    /// The session whose approval the caller enters, for a handed-over
    /// approval.
    pub reviewer: Option<SessionRef>,
}

/// What a recording admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub head: CommitSha,
    pub version: u64,
    pub name: ContentDigest,
    pub moment: Moment,
}

/// Admits and records a head verdict (`instrukt review record`). Every read
/// that may wait — git, delivery's records, the runtime — happens before
/// `state/review.lock` is taken; the transaction reads the commit's state
/// again and decides under the lock.
pub fn record(
    home: &Home,
    env: &instrukt_core::config::Env,
    project: &ReviewProject,
    request: &RecordRequest,
    clock: &dyn Fn() -> Timestamp,
) -> Result<Recorded, ReviewError> {
    let head = project.commit(&request.head)?;
    let Some(caller) = request.caller.clone() else {
        return Err(refused(
            "no session resolves for this caller; record a verdict from the reviewing agent session the selected runtime recognises",
        ));
    };
    let entering = request.reviewer.as_ref().map(|_| caller.clone());
    let reviewer = request.reviewer.clone().unwrap_or(caller);
    if request.report.trim().is_empty() {
        return Err(refused(
            "the report is missing; supply the complete review report with --report",
        ));
    }
    let state = RunnerState::new(home);
    let requests = instrukt_runner::review::requests(&state, &project.slug, &head)?;
    let held = match _store::reader(home)? {
        Some(conn) => _store::held(&conn, &project.slug, &head)?,
        None => Held::Nothing,
    };
    let (offer, check) = _admission::offer(&held, &reviewer, &requests);
    if let Offer::HandedOff(export) = &offer {
        return Err(handed_off(&head, export));
    }
    let answers = match check {
        RequestCheck::Unrequested => None,
        RequestCheck::Answers(number) => Some(number),
        RequestCheck::Refused {
            number,
            standing,
            holder,
        } => {
            let holder = holder
                .map(|session| format!(" and is held by session {session}"))
                .unwrap_or_else(|| ", and no session holds it yet".to_owned());
            return Err(refused(format!(
                "review request {number} for {head} is {standing:?}{holder}; only the session it was handed over to records this commit's verdict"
            )));
        }
    };
    let returned_base = match &entering {
        Some(entering) => handed_over(home, env, &head, &reviewer, entering, request, &held)?,
        None => false,
    };
    match offer {
        Offer::First | Offer::Revision => {}
        Offer::AskTurn => ask_turn(&state, project, &head, &reviewer)?,
        Offer::SettledByAnother(session) => {
            return Err(refused(format!(
                "{head} is settled by session {session}'s verdict; see `instrukt review show {head}`"
            )));
        }
        Offer::HandedOff(export) => return Err(handed_off(&head, &export)),
    }
    let canonical = project.canonical()?;
    let merge_base = project.merge_base(head.as_str(), &canonical)?;
    let Some(pinned) = _admission::pinned_base(&held, &requests, merge_base.as_ref()) else {
        return Err(refused(format!(
            "{head} shares no history with the canonical branch {canonical}; there is no pinned base to review it from"
        )));
    };
    let base = project.commit(&request.base)?;
    if base != pinned && !project.contains(base.as_str(), pinned.as_str())? {
        return Err(refused(format!(
            "--base {base} is neither the pinned base {pinned} nor one of its ancestors; review from {pinned}"
        )));
    }
    let base_on_canonical = project.contains(pinned.as_str(), &canonical)?;
    let commits = project.sessions_in(&pinned, &head)?;
    for session in std::iter::once(&reviewer).chain(entering.as_ref()) {
        let naming: Vec<String> = commits
            .iter()
            .filter(|(_, sessions)| sessions.iter().any(|s| s == &session.to_string()))
            .map(|(sha, _)| sha.to_string())
            .collect();
        if !naming.is_empty() {
            return Err(refused(format!(
                "session {session} authored reviewed commits {}; the review needs a session none of them names",
                naming.join(", ")
            )));
        }
    }
    if entering.is_some() && !_admission::names_commit(&request.report, &base) && !returned_base {
        return Err(refused(format!(
            "the base {base} is named neither in session {reviewer}'s message returning the report nor in the report"
        )));
    }
    let unattributed = commits
        .iter()
        .filter(|(_, sessions)| sessions.is_empty())
        .map(|(sha, _)| sha.clone())
        .collect();
    let recorded_on = instrukt_core::machine::machine_id(home)?.to_string();
    let aged = aged_history(home, project, clock())?;
    let mut db = _store::open(home)?;
    let recorded = db.write(|tx| {
        let now = clock();
        let held_now = _store::held(tx, &project.slug, &head)?;
        if held_now != held {
            return Err(refused(format!(
                "{head}'s verdict changed while this one was offered; see `instrukt review show {head}` and offer it again"
            )));
        }
        let (settlement_id, number, over) = match &held_now {
            Held::Stands {
                settlement,
                current,
            } => (*settlement, current.number + 1, Some(current.moment)),
            Held::Nothing => (_store::settle(tx, &project.slug, &head, now)?, 1, None),
            Held::HandedOff { export } => return Err(handed_off(&head, export)),
        };
        let version = Version {
            number,
            verdict: request.verdict,
            name: ContentDigest::of(&request.report),
            report: request.report.clone(),
            reviewer: reviewer.clone(),
            entered_by: entering.clone(),
            base: pinned.clone(),
            base_on_canonical,
            request: answers,
            unattributed,
            moment: Moment::decided(now, over),
            recorded_at: now,
            recorded_on,
        };
        _store::add_version(tx, settlement_id, &version)?;
        _store::retire(tx, retention_cutoff(now), &aged)?;
        Ok::<_, ReviewError>(Recorded {
            head: head.clone(),
            version: number,
            name: version.name,
            moment: version.moment,
        })
    })?;
    tracing::info!(event = "review_verdict_admitted", project = %project.slug, head = %recorded.head, version = recorded.version, verdict = request.verdict.as_str(), request = ?answers);
    Ok(recorded)
}

/// The facts work establishes beside a handed-over approval's statement:
/// an approving report, no verdict of the commit yet, an entering session
/// other than the named one, and a message the named session authored that
/// names the head together with the report's digest. Returns whether such a
/// message also names the supplied base.
fn handed_over(
    home: &Home,
    env: &instrukt_core::config::Env,
    head: &CommitSha,
    reviewer: &SessionRef,
    entering: &SessionRef,
    request: &RecordRequest,
    held: &Held,
) -> Result<bool, ReviewError> {
    if request.verdict != Verdict::Approve || !_admission::approving(&request.report) {
        return Err(refused(
            "a handed-over result is entered only as an approval whose report ends in the approving verdict line `Verdict: APPROVE`",
        ));
    }
    if held != &Held::Nothing {
        return Err(refused(format!(
            "{head} already has a verdict; a handed-over approval is entered only for a commit that has none"
        )));
    }
    if entering == reviewer {
        return Err(refused(
            "the named session records its own result without --reviewer",
        ));
    }
    let messages = match instrukt_runner::review::conversation(home, env, reviewer) {
        instrukt_runner::review::Conversation::Read { messages, .. } => messages,
        instrukt_runner::review::Conversation::CannotRead(cause) => {
            return Err(refused(format!(
                "session {reviewer}'s conversation cannot be read ({cause}); nothing is recorded"
            )));
        }
    };
    let digest = ContentDigest::of(&request.report);
    let returning: Vec<&str> = messages
        .iter()
        .filter(|message| message.role == instrukt_runner::runtime::conversations::Role::Assistant)
        .map(|message| message.text.as_str())
        .filter(|text| _admission::names_head_and_digest(text, head, digest.as_str()))
        .collect();
    if returning.is_empty() {
        return Err(refused(format!(
            "no message session {reviewer} authored names {head} together with the report's digest {digest}"
        )));
    }
    let base = CommitSha::parse(&request.base).ok();
    Ok(base.is_some_and(|base| {
        returning
            .iter()
            .any(|text| _admission::names_commit(text, &base))
    }))
}

fn handed_off(head: &CommitSha, export: &str) -> ReviewError {
    refused(format!(
        "{head}'s verdict was handed off by export {export}; record on the computer that imported it, or import here a file that carries the commit's verdict"
    ))
}

/// Asks which input opened the reviewing session's turn until the runtime
/// answers or the command's wait has passed.
fn ask_turn(
    state: &RunnerState,
    project: &ReviewProject,
    head: &CommitSha,
    reviewer: &SessionRef,
) -> Result<(), ReviewError> {
    let started = Instant::now();
    loop {
        let answer = instrukt_runner::review::turn_opening(state, &project.slug, head, reviewer)?;
        match _admission::turn_decision(&answer) {
            TurnDecision::Admit => return Ok(()),
            TurnDecision::OpenedByRequest(number) => {
                return Err(refused(format!(
                    "review request {number} for {head} opened the turn session {reviewer} has in progress, and {head} is already settled; that request is answered by the verdict that stands"
                )));
            }
            TurnDecision::NoTurn => {
                return Err(refused(format!(
                    "session {reviewer} has no turn in progress; a revision is recorded within a turn of its session"
                )));
            }
            TurnDecision::AskAgain(cause) => {
                if started.elapsed() >= TURN_WAIT {
                    return Err(refused(format!(
                        "the runtime adapter cannot say which input opened session {reviewer}'s turn ({cause}); nothing is recorded: run the same command with the same report once the runtime answers"
                    )));
                }
                std::thread::sleep(TURN_RETRY);
            }
        }
    }
}

/// The cutoff before which history is retired: the work record's recovery
/// window.
fn retention_cutoff(now: Timestamp) -> Timestamp {
    now.checked_sub(super::_session::RECOVERY_WINDOW)
        .unwrap_or(Timestamp::MIN)
}

/// The commits of `project` whose evidence aged past the retention period,
/// as git shows them to be history: those the canonical branch contains,
/// and those no branch reaches any more. A commit this repository does not
/// hold is reached by no branch.
fn aged_history(
    home: &Home,
    project: &ReviewProject,
    now: Timestamp,
) -> Result<_store::Aged, ReviewError> {
    let mut aged = _store::Aged::default();
    let Some(conn) = _store::reader(home)? else {
        return Ok(aged);
    };
    let canonical = project.canonical()?;
    for commit in _store::aged_commits(&conn, &project.slug, retention_cutoff(now))? {
        if !project.holds(&commit)? {
            aged.unreached.push((project.slug.clone(), commit));
        } else if project.contains(commit.as_str(), &canonical)? {
            aged.contained.push((project.slug.clone(), commit));
        } else if !project.reached(&commit)? {
            aged.unreached.push((project.slug.clone(), commit));
        }
    }
    Ok(aged)
}

/// An exemption offered for recording.
#[derive(Debug, Clone)]
pub struct ExemptRequest {
    pub commit: String,
    pub basis: String,
    pub caller: Option<SessionRef>,
}

/// Admits and records an exemption (`instrukt review exempt`).
pub fn exempt(
    home: &Home,
    project: &ReviewProject,
    request: &ExemptRequest,
    clock: &dyn Fn() -> Timestamp,
) -> Result<Exemption, ReviewError> {
    let commit = project.commit(&request.commit)?;
    if request.basis.trim().is_empty() {
        return Err(refused(
            "the basis is missing; state with --basis why the commit owes no independent review",
        ));
    }
    let canonical = project.canonical()?;
    if project.contains(commit.as_str(), &canonical)? {
        return Err(refused(format!(
            "the canonical branch {canonical} already contains {commit}; an exemption is recorded only for a commit it does not contain"
        )));
    }
    let authors = project.sessions_of(&commit)?;
    let caller = request.caller.as_ref().map(ToString::to_string);
    if !authors.is_empty() && !caller.as_ref().is_some_and(|c| authors.contains(c)) {
        return Err(refused(format!(
            "{commit} names its authoring session {}; only that session's exemption is admitted",
            authors.join(", ")
        )));
    }
    let exemption = Exemption {
        commit: commit.clone(),
        basis: request.basis.clone(),
        declared_by: request.caller.clone(),
        names_session: !authors.is_empty(),
        recorded_at: clock(),
        recorded_on: instrukt_core::machine::machine_id(home)?.to_string(),
    };
    let aged = aged_history(home, project, clock())?;
    let mut db = _store::open(home)?;
    db.write(|tx| {
        if let Some(held) = _store::exemption(tx, &project.slug, &commit)? {
            return Err(refused(format!(
                "{commit} is already exempted ({}); see `instrukt review show {commit}`",
                held.basis
            )));
        }
        _store::exempt(tx, &project.slug, &exemption)?;
        _store::retire(tx, retention_cutoff(exemption.recorded_at), &aged)?;
        Ok::<_, ReviewError>(())
    })?;
    tracing::info!(event = "review_exemption_admitted", project = %project.slug, commit = %commit, names_session = exemption.names_session);
    Ok(exemption)
}

/// What this computer holds of `commit`'s review evidence, read without a
/// lock.
pub fn evidence(
    home: &Home,
    project: &ProjectSlug,
    commit: &CommitSha,
) -> Result<CommitEvidence, ReviewError> {
    let mut evidence = CommitEvidence::default();
    if let Some(conn) = _store::reader(home)? {
        evidence.settlements = _store::settlements(&conn, project, commit)?;
        evidence.exemption = _store::exemption(&conn, project, commit)?;
    }
    if let Some(conn) = _published::reader(home)? {
        evidence.deliveries = _published::deliveries(&conn, project, commit)?;
        evidence.recognised = _published::recognised(&conn, project, commit)?;
    }
    evidence.derived = derived_approval(home, project, commit)?;
    Ok(evidence)
}

/// The derived approval of `candidate`: the one the publication store holds,
/// withdrawn or not, else the one an import brought (`github-apps`, Review
/// evidence storage: whether a derived approval was withdrawn is read from
/// the publication store by every reader).
fn derived_approval(
    home: &Home,
    project: &ProjectSlug,
    candidate: &CommitSha,
) -> Result<Option<Derived>, ReviewError> {
    if let Some(conn) = _published::reader(home)?
        && let Some(approval) = _published::derived_of(&conn, project, candidate)?
    {
        return Ok(Some(approval));
    }
    match _store::reader(home)? {
        Some(conn) => _store::imported_approval(&conn, project, candidate),
        None => Ok(None),
    }
}

/// Every derived approval of `project` that was not withdrawn: those the
/// tick recorded, and those imports brought that the publication store
/// holds nothing of.
fn standing_derived(home: &Home, project: &ProjectSlug) -> Result<Vec<Derived>, ReviewError> {
    let (mut standing, held) = match _published::reader(home)? {
        Some(conn) => (
            _published::standing_derived(&conn, project)?,
            _published::candidates(&conn, project)?,
        ),
        None => Default::default(),
    };
    if let Some(conn) = _store::reader(home)? {
        standing.extend(
            _store::imported_approvals(&conn, project)?
                .into_iter()
                .filter(|approval| !held.contains(&approval.candidate)),
        );
    }
    standing.sort_by(|a, b| a.candidate.cmp(&b.candidate));
    Ok(standing)
}

/// The repository `root` of `project`, as review evidence reads it.
pub fn project(slug: ProjectSlug, root: &Path) -> ReviewProject {
    ReviewProject {
        slug,
        root: root.to_path_buf(),
    }
}

/// Work's answers for automatic review, as the runner asks them.
pub struct ReviewWork {
    home: Home,
    access: std::sync::Arc<super::GitHubAccess>,
    records: Option<std::sync::Arc<dyn crate::evidence::WorkEvidence>>,
}

impl ReviewWork {
    /// Work's answers, reading todos' records through `records` when the
    /// todos module is available.
    pub fn new(
        home: Home,
        access: std::sync::Arc<super::GitHubAccess>,
        records: Option<std::sync::Arc<dyn crate::evidence::WorkEvidence>>,
    ) -> Self {
        Self {
            home,
            access,
            records,
        }
    }
}

impl instrukt_runner::interfaces::AutomaticReview for ReviewWork {
    fn readiness(
        &self,
        repository: &str,
        now: Timestamp,
    ) -> instrukt_core::interfaces::ReviewReadiness {
        use instrukt_core::interfaces::AutomaticReviewReadiness as _;
        self.access.review_readiness(repository, now)
    }

    fn identities(&self) -> Result<instrukt_runner::interfaces::ReviewIdentities, String> {
        self.access
            .review_identities()
            .map_err(|error| error.to_string())
    }

    fn todo_pulls(&self, project: &ProjectSlug) -> Result<std::collections::BTreeSet<u64>, String> {
        let read = instrukt_core::state_db::StateRead::open(&self.home)
            .map_err(|error| error.to_string())?;
        let Some(db) = read else {
            return Ok(std::collections::BTreeSet::new());
        };
        let mut statement = db
            .conn()
            .prepare(
                "SELECT r.pull_number FROM reflections r JOIN todos t ON t.id = r.todo_id
                 WHERE t.project = ?1 AND r.pull_number IS NOT NULL",
            )
            .map_err(|error| error.to_string())?;
        statement
            .query_map([project.as_str()], |row| row.get::<_, u64>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    fn carries_verdict(&self, project: &ProjectSlug, head: &CommitSha) -> Result<bool, String> {
        let evidence = evidence(&self.home, project, head).map_err(|error| error.to_string())?;
        Ok(evidence.carries_verdict())
    }

    fn needs_work(&self, project: &ProjectSlug, commit: &CommitSha) -> Result<bool, String> {
        Ok(evidence(&self.home, project, commit)
            .map_err(|error| error.to_string())?
            .current()
            .is_some_and(|version| version.verdict == Verdict::NeedsWork))
    }

    fn candidate(
        &self,
        project: &instrukt_runner::interfaces::ProjectRef,
        repository: &str,
        candidate: &CommitSha,
        pulls: &[u64],
        now: Timestamp,
        clock: &dyn Fn() -> Timestamp,
    ) -> Result<instrukt_runner::interfaces::CandidateFinding, String> {
        candidate_finding(self, project, repository, candidate, pulls, now, clock)
            .map_err(|error| error.to_string())
    }

    fn publish(
        &self,
        project: &ProjectSlug,
        repository: &str,
        eligible: &[(u64, CommitSha)],
        candidate: Option<&CommitSha>,
        now: Timestamp,
        clock: &dyn Fn() -> Timestamp,
    ) -> Result<(), String> {
        publish(
            &self.home,
            &self.access,
            project,
            repository,
            eligible,
            candidate,
            now,
            clock,
        )
        .map_err(|error| error.to_string())
    }
}

/// Publishes the current verdict of every commit whose head verdict stands
/// here, and recognises what the reviewer bot already published for each
/// eligible head this computer holds no evidence of (`github-apps`, Verdict
/// publication). The tick's alone; it retires the publication store's
/// history, keeping what concerns the active `candidate`. Its credential is
/// issued on `clock`; `now` dates its records.
#[allow(
    clippy::too_many_arguments,
    reason = "the tick's publication pass borrows each input from its own owner"
)]
fn publish(
    home: &Home,
    access: &super::GitHubAccess,
    project: &ProjectSlug,
    repository: &str,
    eligible: &[(u64, CommitSha)],
    candidate: Option<&CommitSha>,
    now: Timestamp,
    clock: &dyn Fn() -> Timestamp,
) -> Result<(), ReviewError> {
    let gh = access
        .review_client(repository, clock)
        .map_err(|error| refused(error.to_string()))?;
    let reviewer = access
        .review_identities()
        .map_err(|error| refused(error.to_string()))?
        .reviewer;
    let publication = _recognition::Publication {
        home,
        gh: &gh,
        project,
        repository,
        reviewer: &reviewer,
        now,
    };
    let pulls_at = |commit: &CommitSha| -> Vec<u64> {
        eligible
            .iter()
            .filter(|(_, head)| head == commit)
            .map(|(number, _)| *number)
            .collect()
    };
    let standing = match _store::reader(home)? {
        Some(conn) => _store::standing(&conn, project)?,
        None => Vec::new(),
    };
    for (commit, version) in &standing {
        let target = _recognition::Target {
            published: _publication::Published {
                verdict: version.verdict,
                name: version.name.clone(),
                moment: version.moment,
            },
            body: Some(_publication::body(
                &version.name,
                version.moment,
                &version.report,
                commit,
            )),
            status_first: false,
        };
        publication.deliver(commit, &target, &pulls_at(commit))?;
    }
    let mut current: Vec<CommitSha> = standing.iter().map(|(commit, _)| commit.clone()).collect();
    current.extend(candidate.cloned());
    let derived = standing_derived(home, project)?;
    for approval in &derived {
        // Only one GitHub already shows as the status is completed here;
        // the candidate's own pass publishes one GitHub does not show.
        let shown = evidence(home, project, &approval.candidate)?
            .deliveries
            .iter()
            .any(|d| {
                d.destination == Destination::Status
                    && d.state == DeliveryState::Shown
                    && d.name == approval.name
            });
        if shown {
            publication.deliver(
                &approval.candidate,
                &derived_target(approval),
                &pulls_at(&approval.candidate),
            )?;
        }
    }
    let mut heads: Vec<&CommitSha> = eligible.iter().map(|(_, head)| head).collect();
    heads.sort();
    heads.dedup();
    for commit in heads {
        // A commit whose verdict this computer holds is published from it,
        // never recognised.
        if standing.iter().any(|(held, _)| held == commit)
            || derived.iter().any(|approval| &approval.candidate == commit)
        {
            continue;
        }
        current.push(commit.clone());
        let pulls = pulls_at(commit);
        let recognised = match publication.recognise(commit, &pulls) {
            Ok(recognised) => recognised,
            Err(error) => {
                tracing::warn!(event = "review_recognition_failed", project = %project, commit = %commit, error = %error);
                continue;
            }
        };
        let Some(recognised) = recognised else {
            continue;
        };
        let mut db = _published::open(home)?;
        db.write(|tx| _published::recognise(tx, project, commit, &recognised))?;
        drop(db);
        tracing::info!(event = "review_verdict_recognised", project = %project, commit = %commit, verdict = recognised.published.verdict.as_str());
        let target = _recognition::Target {
            published: recognised.published,
            body: recognised.body,
            status_first: false,
        };
        publication.deliver(commit, &target, &pulls)?;
    }
    let mut db = _published::open(home)?;
    db.write(|tx| _published::retire(tx, project, retention_cutoff(now), &current))
}

/// What an export handed off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    pub digest: String,
    pub verdicts: usize,
    pub exemptions: usize,
    pub derived: usize,
}

/// Writes one snapshot of `project`'s review evidence that stands here to
/// `to` and hands its verdicts off (`instrukt review export`). The file is
/// complete on disk before the hand-off commits, all under
/// `state/review.lock`.
pub fn export(
    home: &Home,
    project: &ProjectSlug,
    to: &Path,
    now: Timestamp,
) -> Result<Exported, ReviewError> {
    let derived = standing_derived(home, project)?
        .into_iter()
        .map(|approval| _transfer::ExportedDerived {
            candidate: approval.candidate.to_string(),
            canonical: approval.canonical,
            coverage: approval.coverage,
            name: approval.name.to_string(),
            moment: approval.moment.text(),
        })
        .collect();
    let mut db = _store::open(home)?;
    let standing = _store::standing(db.conn(), project)?;
    let exemptions = _store::exemptions(db.conn(), project)?;
    let export = _transfer::Export {
        project: project.to_string(),
        computer: instrukt_core::machine::machine_id(home)?.to_string(),
        written_at: now.to_string(),
        verdicts: standing
            .iter()
            .map(|(head, version)| _transfer::ExportedVerdict {
                head: head.to_string(),
                verdict: version.verdict.as_str().to_owned(),
                name: version.name.to_string(),
                report: version.report.clone(),
                reviewer: version.reviewer.to_string(),
                entered_by: version.entered_by.as_ref().map(ToString::to_string),
                base: version.base.to_string(),
                base_on_canonical: version.base_on_canonical,
                request: version.request,
                unattributed: version
                    .unattributed
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                moment: version.moment.text(),
                recorded_at: version.recorded_at.to_string(),
                recorded_on: version.recorded_on.clone(),
            })
            .collect(),
        exemptions: exemptions
            .iter()
            .map(|exemption| _transfer::ExportedExemption {
                commit: exemption.commit.to_string(),
                basis: exemption.basis.clone(),
                declared_by: exemption.declared_by.as_ref().map(ToString::to_string),
                names_session: exemption.names_session,
                recorded_at: exemption.recorded_at.to_string(),
                recorded_on: exemption.recorded_on.clone(),
            })
            .collect(),
        derived,
    };
    let (text, digest) =
        _transfer::write(&export).map_err(|error| ReviewError::Record(error.to_string()))?;
    {
        use std::os::unix::fs::PermissionsExt as _;
        instrukt_core::fsx::atomic_write_with_permissions(
            to,
            text.as_bytes(),
            std::fs::Permissions::from_mode(0o600),
        )
        .map_err(|error| {
            refused(format!(
                "cannot write the export to {}: {error}",
                to.display()
            ))
        })?;
    }
    db.write(|tx| _store::hand_off(tx, project, &digest, now))?;
    tracing::info!(event = "review_evidence_exported", project = %project, export = %digest, verdicts = export.verdicts.len());
    Ok(Exported {
        digest,
        verdicts: export.verdicts.len(),
        exemptions: export.exemptions.len(),
        derived: export.derived.len(),
    })
}

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imported {
    pub standing: usize,
    pub unchanged: usize,
    pub set_aside: usize,
}

fn version_of(verdict: &_transfer::ExportedVerdict) -> Result<Version, ReviewError> {
    let bad = |what: &str| {
        refused(format!(
            "the export's verdict of {} has an unreadable {what}",
            verdict.head
        ))
    };
    let session = |text: &str| SessionRef::parse(text).map_err(|_| bad("session"));
    Ok(Version {
        number: 1,
        verdict: Verdict::parse(&verdict.verdict).ok_or_else(|| bad("verdict"))?,
        name: ContentDigest::new(verdict.name.clone()),
        report: verdict.report.clone(),
        reviewer: session(&verdict.reviewer)?,
        entered_by: verdict.entered_by.as_deref().map(session).transpose()?,
        base: CommitSha::parse(&verdict.base).map_err(|_| bad("base"))?,
        base_on_canonical: verdict.base_on_canonical,
        request: verdict.request,
        unattributed: verdict
            .unattributed
            .iter()
            .map(|sha| CommitSha::parse(sha).map_err(|_| bad("commit")))
            .collect::<Result<_, _>>()?,
        moment: Moment::parse(&verdict.moment).ok_or_else(|| bad("moment"))?,
        recorded_at: verdict
            .recorded_at
            .parse()
            .map_err(|_| bad("recording time"))?,
        recorded_on: verdict.recorded_on.clone(),
    })
}

/// A derived approval an export carries, read as this computer keeps it.
fn derived_of_export(
    approval: &_transfer::ExportedDerived,
    now: Timestamp,
) -> Result<Derived, ReviewError> {
    let bad = |what: &str| {
        refused(format!(
            "the export's derived approval of {} has an unreadable {what}",
            approval.candidate
        ))
    };
    let canonical = CommitSha::parse(&approval.canonical).map_err(|_| bad("canonical head"))?;
    if ContentDigest::of(&approval.coverage).as_str() != approval.name {
        return Err(bad("name: it is not the digest of its coverage"));
    }
    Ok(Derived {
        candidate: CommitSha::parse(&approval.candidate).map_err(|_| bad("candidate"))?,
        canonical: canonical.to_string(),
        coverage: approval.coverage.clone(),
        name: ContentDigest::new(approval.name.clone()),
        moment: Moment::parse(&approval.moment).ok_or_else(|| bad("moment"))?,
        recorded_at: now,
        withdrawn: None,
        imported: true,
    })
}

/// Takes in the export at `file` (`instrukt review import`), commit by
/// commit, under `state/review.lock`. Two different verdicts of one commit
/// refuse the import whole unless `statement` says which stands.
pub fn import(
    home: &Home,
    project: &ProjectSlug,
    file: &Path,
    statement: Option<Statement>,
    now: Timestamp,
) -> Result<Imported, ReviewError> {
    let text = std::fs::read_to_string(file)
        .map_err(|error| refused(format!("cannot read {}: {error}", file.display())))?;
    let (export, digest) = _transfer::read(&text).map_err(|why| {
        refused(format!(
            "{} is not an export this instrukt takes in ({why:?}); nothing changed",
            file.display()
        ))
    })?;
    if export.project != project.as_str() {
        return Err(refused(format!(
            "{} is an export of project {}, not {project}; nothing changed",
            file.display(),
            export.project
        )));
    }
    let versions = export
        .verdicts
        .iter()
        .map(|verdict| {
            Ok((
                CommitSha::parse(&verdict.head).map_err(|_| {
                    refused(format!("the export names `{}` as a commit", verdict.head))
                })?,
                version_of(verdict)?,
            ))
        })
        .collect::<Result<Vec<_>, ReviewError>>()?;
    let approvals = export
        .derived
        .iter()
        .map(|approval| derived_of_export(approval, now))
        .collect::<Result<Vec<_>, ReviewError>>()?;
    let mut db = _store::open(home)?;
    let imported = db.write(|tx| {
        let mut done = Imported::default();
        let mut two = Vec::new();
        let mut plan = Vec::new();
        for (head, version) in &versions {
            let held = _store::held(tx, project, head)?;
            let current = match &held {
                Held::Stands { settlement, current } => Some((*settlement, current.as_ref())),
                Held::Nothing | Held::HandedOff { .. } => None,
            };
            let rule = _transfer::rule(
                current.map(|(_, v)| v),
                version.verdict,
                version.name.as_str(),
                version.moment,
            );
            if rule == _transfer::Rule::Two {
                two.push(head.to_string());
            }
            plan.push((head, version, current.map(|(id, v)| (id, v.clone())), rule));
        }
        if !two.is_empty() && statement.is_none() {
            return Err(refused(format!(
                "this computer and the export hold two different verdicts of {}; nothing changed: state which stands with --standing imported or --standing held",
                two.join(", ")
            )));
        }
        let cause = |which: &str| format!("import {digest} with --standing {which}");
        for (head, version, current, rule) in plan {
            match (rule, current, statement) {
                (_transfer::Rule::Stands, _, _) => {
                    let id = _store::settle(tx, project, head, now)?;
                    _store::add_version(tx, id, version)?;
                    done.standing += 1;
                }
                (_transfer::Rule::Same { later }, Some((id, held)), _) => {
                    if let Some(later) = later {
                        _store::move_moment(tx, id, held.number, later)?;
                    }
                    done.unchanged += 1;
                }
                (_transfer::Rule::Two, Some((id, held)), Some(Statement::Imported)) => {
                    _store::set_aside(tx, id, &cause("imported"), now)?;
                    let standing = _store::settle(tx, project, head, now)?;
                    let mut version = version.clone();
                    version.moment = _transfer::chosen_moment(version.moment, held.moment, now);
                    _store::add_version(tx, standing, &version)?;
                    done.set_aside += 1;
                }
                (_transfer::Rule::Two, Some((id, held)), Some(Statement::Held)) => {
                    if !_store::set_aside_named(tx, project, head, version.name.as_str())? {
                        let aside = _store::arrive_set_aside(tx, project, head, &cause("held"), now)?;
                        _store::add_version(tx, aside, version)?;
                    }
                    let moment = _transfer::chosen_moment(held.moment, version.moment, now);
                    if moment != held.moment {
                        _store::move_moment(tx, id, held.number, moment)?;
                    }
                    done.set_aside += 1;
                }
                _ => {}
            }
        }
        for exemption in &export.exemptions {
            let commit = CommitSha::parse(&exemption.commit)
                .map_err(|_| refused(format!("the export names `{}` as a commit", exemption.commit)))?;
            if _store::exemption(tx, project, &commit)?.is_some() {
                continue;
            }
            _store::exempt(
                tx,
                project,
                &Exemption {
                    commit,
                    basis: exemption.basis.clone(),
                    declared_by: exemption
                        .declared_by
                        .as_deref()
                        .map(SessionRef::parse)
                        .transpose()
                        .map_err(|_| refused("the export names an unreadable declaring session"))?,
                    names_session: exemption.names_session,
                    recorded_at: exemption
                        .recorded_at
                        .parse()
                        .map_err(|_| refused("the export holds an unreadable exemption time"))?,
                    recorded_on: exemption.recorded_on.clone(),
                },
            )?;
        }
        for approval in &approvals {
            _store::import_approval(tx, project, approval)?;
        }
        Ok::<_, ReviewError>(done)
    })?;
    tracing::info!(event = "review_evidence_imported", project = %project, export = %digest, standing = imported.standing, set_aside = imported.set_aside);
    Ok(imported)
}

/// A derived approval as the verdict its publication delivers.
fn derived_target(approval: &Derived) -> _recognition::Target {
    _recognition::Target {
        published: _publication::Published {
            verdict: Verdict::Approve,
            name: approval.name.clone(),
            moment: approval.moment,
        },
        body: Some(_publication::body(
            &approval.name,
            approval.moment,
            &approval.coverage,
            &approval.candidate,
        )),
        status_first: true,
    }
}

/// The evidence candidate coverage reads, as work holds it.
struct CoverageReader<'a> {
    work: &'a ReviewWork,
    project: &'a instrukt_runner::interfaces::ProjectRef,
}

impl CoverageReader<'_> {
    /// The todo a commit's `Todo:` trailer names.
    fn todo_of(&self, commit: &CommitSha) -> Result<Option<crate::evidence::TodoEvidence>, String> {
        let Some(records) = &self.work.records else {
            return Ok(None);
        };
        let named = git::run(
            &self.project.root,
            [
                "log",
                "-1",
                "--format=%(trailers:key=Todo,valueonly)",
                commit.as_str(),
            ],
        )
        .map_err(|e| e.to_string())?;
        let Some(slug) = named
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .and_then(|slug| instrukt_core::ids::TodoSlug::parse(slug).ok())
        else {
            return Ok(None);
        };
        records
            .evidence(self.project, &slug)
            .map_err(|e| e.to_string())
    }
}

/// An independent approval a todo's record binds to an exact head.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordApproval {
    head: CommitSha,
    report: String,
    /// For a build-review approval, its stamp's record order: how the
    /// todo's reflection names the verdicts reviewed on its pull request.
    /// None for a reviewed preparation head.
    build: Option<u64>,
}

impl RecordApproval {
    /// The pull request this approval cites: for a build-review approval,
    /// the todo's pull request once `reflected` — the pull request and the
    /// build verdicts reviewed there — holds its own verdict, whatever
    /// became of any other; never for a reviewed preparation head.
    fn cited_pull(&self, reflected: Option<&(u64, Vec<u64>)>) -> Option<u64> {
        match (self.build, reflected) {
            (Some(sequence), Some((pull, reviews))) if reviews.contains(&sequence) => Some(*pull),
            _ => None,
        }
    }

    /// The covering item's reference (`github-apps`, Verdict publication):
    /// the todo, the approval and its report's digest, and the pull request
    /// it cites.
    fn reference(
        &self,
        todo: &instrukt_core::ids::TodoSlug,
        reflected: Option<&(u64, Vec<u64>)>,
    ) -> String {
        let (head, report) = (&self.head, &self.report);
        match (self.build, self.cited_pull(reflected)) {
            (Some(_), Some(pull)) => format!(
                "build-review approval of {head} in todo {todo} ({report}), reflected as a review on pull request #{pull}"
            ),
            (Some(_), None) => format!("build-review approval of {head} in todo {todo} ({report})"),
            (None, _) => format!("preparation-head approval of {head} in todo {todo} ({report})"),
        }
    }
}

/// The head a todo's record approves last, when the last build verdict or
/// preparation-head review of that head among `facts` approves it.
fn record_approval<'a>(
    stamps: impl IntoIterator<Item = (u64, &'a crate::evidence::StampFact)>,
    commit: Option<&CommitSha>,
) -> Option<RecordApproval> {
    use crate::evidence::{BuildVerdict, ReviewVerdict, StampFact};
    let mut last: Option<(RecordApproval, bool)> = None;
    for (sequence, fact) in stamps {
        let (head, report, approves, build) = match fact {
            StampFact::BuildVerdict {
                verdict,
                target,
                report,
            } => (
                target,
                report,
                *verdict == BuildVerdict::Approve,
                Some(sequence),
            ),
            StampFact::PreparationHeadReviewed {
                verdict,
                head,
                report,
            } => (head, report, *verdict == ReviewVerdict::Approve, None),
            _ => continue,
        };
        if commit.is_none_or(|commit| commit == head) {
            last = Some((
                RecordApproval {
                    head: head.clone(),
                    report: report.to_string(),
                    build,
                },
                approves,
            ));
        }
    }
    last.filter(|(_, approves)| *approves)
        .map(|(approval, _)| approval)
}

impl CoverageReader<'_> {
    /// The todo's pull request with the build verdicts reviewed there, by
    /// their stamps' record order, when it has one.
    fn reflection(&self, todo: &super::TodoId) -> Result<Option<(u64, Vec<u64>)>, String> {
        use instrukt_core::state_db::rusqlite::OptionalExtension as _;
        let Some(db) = instrukt_core::state_db::StateRead::open(&self.work.home)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        db.conn()
            .query_row(
                "SELECT pull_number, reviews FROM reflections
                 WHERE todo_id = ?1 AND pull_number IS NOT NULL",
                [todo.as_str()],
                |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .map(|(pull, reviews)| {
                serde_json::from_str::<Vec<u64>>(&reviews)
                    .map(|reviews| (pull, reviews))
                    .map_err(|error| error.to_string())
            })
            .transpose()
    }
}

impl crate::workflow::CoverageEvidence for CoverageReader<'_> {
    fn approved(&self, commit: &CommitSha) -> Result<Option<crate::workflow::Approved>, String> {
        let held =
            evidence(&self.work.home, &self.project.slug, commit).map_err(|e| e.to_string())?;
        if let Some(version) = held.current()
            && version.verdict == Verdict::Approve
            && version.base_on_canonical
        {
            return Ok(Some(crate::workflow::Approved {
                reference: format!(
                    "head verdict {} of {commit} by {}",
                    version.name, version.reviewer
                ),
                todo: None,
            }));
        }
        let Some(record) = self.todo_of(commit)? else {
            return Ok(None);
        };
        let Some(approval) = record_approval(
            record.stamps.iter().map(|s| (s.sequence, &s.fact)),
            Some(commit),
        ) else {
            return Ok(None);
        };
        let reflected = match approval.build {
            Some(_) => self.reflection(&record.id)?,
            None => None,
        };
        Ok(Some(crate::workflow::Approved {
            reference: approval.reference(&record.slug, reflected.as_ref()),
            todo: Some(record.slug.clone()),
        }))
    }

    fn exempted(&self, commit: &CommitSha) -> Result<Option<String>, String> {
        let held =
            evidence(&self.work.home, &self.project.slug, commit).map_err(|e| e.to_string())?;
        Ok(held.exemption.map(|exemption| {
            format!(
                "exemption of {commit}: {} (declared by {})",
                exemption.basis,
                exemption
                    .declared_by
                    .map_or_else(|| "no resolved session".to_owned(), |s| s.to_string())
            )
        }))
    }

    fn delivery_head(
        &self,
        todo: &instrukt_core::ids::TodoSlug,
    ) -> Result<Option<CommitSha>, String> {
        let Some(records) = &self.work.records else {
            return Ok(None);
        };
        Ok(records
            .evidence(self.project, todo)
            .map_err(|e| e.to_string())?
            .and_then(|record| {
                record_approval(record.stamps.iter().map(|s| (s.sequence, &s.fact)), None)
            })
            .map(|approval| approval.head))
    }

    fn red_promotion(&self, head: &CommitSha) -> Result<Option<CommitSha>, String> {
        let state = RunnerState::new(&self.work.home);
        if !instrukt_runner::review::observed_red(&state, &self.project.slug, head)
            .map_err(|e| e.to_string())?
        {
            return Ok(None);
        }
        let held =
            evidence(&self.work.home, &self.project.slug, head).map_err(|e| e.to_string())?;
        if let Some(version) = held.current() {
            return Ok(Some(version.base.clone()));
        }
        let derived = derived_approval(&self.work.home, &self.project.slug, head)
            .map_err(|e| e.to_string())?;
        Ok(derived.and_then(|d| CommitSha::parse(&d.canonical).ok()))
    }
}

/// Work's part of a candidate's coverage (`integration-gate`, Review
/// coverage of a candidate): the candidate's own verdict settles it;
/// otherwise workflow derives coverage, and the derived approval is
/// recorded and published, or withdrawn while GitHub does not show it.
fn candidate_finding(
    work: &ReviewWork,
    project: &instrukt_runner::interfaces::ProjectRef,
    repository: &str,
    candidate: &CommitSha,
    pulls: &[u64],
    now: Timestamp,
    clock: &dyn Fn() -> Timestamp,
) -> Result<instrukt_runner::interfaces::CandidateFinding, ReviewError> {
    use instrukt_runner::interfaces::CandidateFinding;
    let held = evidence(&work.home, &project.slug, candidate)?;
    if held
        .current()
        .is_some_and(|version| version.base_on_canonical)
    {
        return Ok(CandidateFinding::Settled);
    }
    let recorded = derived_approval(&work.home, &project.slug, candidate)?;
    let shown = |approval: &Derived| {
        held.deliveries.iter().any(|d| {
            d.destination == Destination::Status
                && d.state == DeliveryState::Shown
                && d.name == approval.name
        })
    };
    if let Some(approval) = recorded.as_ref().filter(|a| a.withdrawn.is_none())
        && shown(approval)
    {
        return Ok(CandidateFinding::Settled);
    }
    let canonical_branch = git::default_branch(&project.root, REMOTE)?;
    let canonical = format!("refs/remotes/{REMOTE}/{canonical_branch}");
    let reader = CoverageReader { work, project };
    match crate::workflow::coverage(&project.root, candidate, &canonical, &reader) {
        crate::workflow::Coverage::CannotAnswer(cause) => Ok(CandidateFinding::CannotAnswer(cause)),
        crate::workflow::Coverage::Uncovered(commit) => {
            if let Some(approval) = recorded.as_ref().filter(|a| a.withdrawn.is_none()) {
                let mut db = _published::open(&work.home)?;
                db.write(|tx| _published::withdraw(tx, &project.slug, approval, &commit, now))?;
                tracing::info!(event = "review_derived_approval_withdrawn", project = %project.slug, candidate = %candidate, uncovered = %commit);
            }
            Ok(CandidateFinding::Owed)
        }
        crate::workflow::Coverage::Covered(items) => {
            let based = git::run(
                &project.root,
                ["merge-base", candidate.as_str(), canonical.as_str()],
            )?;
            let coverage = serde_json::to_string_pretty(&serde_json::json!({
                "candidate": candidate.as_str(),
                "canonical": based.trim(),
                "covering": items.iter().map(|item| serde_json::json!({
                    "commit": item.commit.as_str(),
                    "kind": item.kind.as_str(),
                    "evidence": item.reference,
                })).collect::<Vec<_>>(),
            }))
            .map_err(|e| ReviewError::Record(e.to_string()))?;
            let approval = Derived {
                candidate: candidate.clone(),
                canonical: based.trim().to_owned(),
                name: ContentDigest::of(&coverage),
                coverage,
                moment: Moment::decided(now, None),
                recorded_at: now,
                withdrawn: None,
                imported: false,
            };
            let mut db = _published::open(&work.home)?;
            db.write(|tx| _published::record_derived(tx, &project.slug, &approval))?;
            drop(db);
            tracing::info!(event = "review_derived_approval_recorded", project = %project.slug, candidate = %candidate, items = items.len());
            let gh = work
                .access
                .review_client(repository, clock)
                .map_err(|error| refused(error.to_string()))?;
            let reviewer = work
                .access
                .review_identities()
                .map_err(|error| refused(error.to_string()))?
                .reviewer;
            _recognition::Publication {
                home: &work.home,
                gh: &gh,
                project: &project.slug,
                repository,
                reviewer: &reviewer,
                now,
            }
            .deliver(candidate, &derived_target(&approval), pulls)?;
            Ok(CandidateFinding::Covered)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary home and a project repository whose canonical branch is
    /// `main`, with one commit on a side branch the canonical branch does
    /// not contain.
    struct Fixture {
        _dir: tempfile::TempDir,
        home: Home,
        project: ReviewProject,
        side: CommitSha,
    }

    fn git(root: &Path, args: &[&str]) -> String {
        git::run(root, args).unwrap()
    }

    fn commit(root: &Path, file: &str, message: &str) -> CommitSha {
        std::fs::write(root.join(file), message).unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", message]);
        CommitSha::parse(&git(root, &["rev-parse", "HEAD"])).unwrap()
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.name", "Test"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "commit.gpgsign", "false"],
            vec!["config", "init.defaultBranch", "main"],
        ] {
            git(&root, &args);
        }
        commit(&root, "base.txt", "base");
        git(&root, &["checkout", "-qb", "side"]);
        let side = commit(&root, "side.txt", "side");
        git(&root, &["checkout", "-q", "main"]);
        let home = Home::at_user_home(dir.path().join("user"));
        std::fs::create_dir_all(home.root()).unwrap();
        Fixture {
            project: project(ProjectSlug::parse("demo").unwrap(), &root),
            home,
            _dir: dir,
            side,
        }
    }

    fn sha(c: char) -> CommitSha {
        CommitSha::parse(&c.to_string().repeat(40)).unwrap()
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn version(at: Timestamp) -> Version {
        Version {
            number: 1,
            verdict: Verdict::Approve,
            name: ContentDigest::of("report"),
            report: "report".into(),
            reviewer: SessionRef::parse("t3:reviewer").unwrap(),
            entered_by: None,
            base: sha('b'),
            base_on_canonical: true,
            request: None,
            unattributed: Vec::new(),
            moment: Moment::decided(at, None),
            recorded_at: at,
            recorded_on: "elsewhere".into(),
        }
    }

    /// A head verdict of `head` recorded and last changed at `at`.
    fn settled(fixture: &Fixture, head: &CommitSha, at: Timestamp) {
        let mut db = _store::open(&fixture.home).unwrap();
        db.write(|tx| {
            let id = _store::settle(tx, &fixture.project.slug, head, at)?;
            _store::add_version(tx, id, &version(at))
        })
        .unwrap();
    }

    #[test]
    fn aged_evidence_of_a_commit_the_repository_does_not_hold_leaves_recording_working() {
        let fixture = fixture();
        settled(&fixture, &sha('d'), at("2026-10-01T00:00:00Z"));
        let exempted = exempt(
            &fixture.home,
            &fixture.project,
            &ExemptRequest {
                commit: fixture.side.to_string(),
                basis: "a typo fix".into(),
                caller: None,
            },
            &|| at("2026-11-24T00:00:00Z"),
        );
        assert!(exempted.is_ok(), "{exempted:?}");
        let held = evidence(&fixture.home, &fixture.project.slug, &sha('d')).unwrap();
        assert_eq!(
            held.current().map(|v| v.name.clone()),
            Some(ContentDigest::of("report")),
            "evidence of a commit this repository does not hold is kept"
        );
    }

    #[test]
    fn exempting_a_commit_the_repository_does_not_hold_is_the_repository_refusal() {
        let fixture = fixture();
        let exempted = exempt(
            &fixture.home,
            &fixture.project,
            &ExemptRequest {
                commit: sha('1').to_string(),
                basis: "a typo fix".into(),
                caller: None,
            },
            &|| at("2026-11-24T00:00:00Z"),
        );
        assert!(
            matches!(exempted, Err(ReviewError::Refused(_))),
            "{exempted:?}"
        );
    }

    #[test]
    fn recording_a_head_that_names_no_commit_is_the_repository_refusal() {
        let fixture = fixture();
        let tree = git(&fixture.project.root, &["rev-parse", "HEAD^{tree}"]);
        let resolved = fixture.project.commit(&tree);
        assert!(
            matches!(resolved, Err(ReviewError::Refused(_))),
            "{resolved:?}"
        );
        let recorded = record(
            &fixture.home,
            &instrukt_core::config::Env::default(),
            &fixture.project,
            &RecordRequest {
                head: tree,
                base: fixture.side.to_string(),
                verdict: Verdict::Approve,
                report: "report".into(),
                caller: Some(SessionRef::parse("t3:reviewer").unwrap()),
                reviewer: None,
            },
            &|| at("2026-11-24T00:00:00Z"),
        );
        assert!(
            matches!(recorded, Err(ReviewError::Refused(_))),
            "{recorded:?}"
        );
    }

    /// A derived approval of `candidate` as the tick records it at `at`.
    fn tick_derived(candidate: &CommitSha, at: Timestamp) -> Derived {
        let coverage = format!("{{\"candidate\": \"{candidate}\"}}");
        Derived {
            candidate: candidate.clone(),
            canonical: sha('b').to_string(),
            name: ContentDigest::of(&coverage),
            coverage,
            moment: Moment::decided(at, None),
            recorded_at: at,
            withdrawn: None,
            imported: false,
        }
    }

    /// Exports `project` from `from` and imports the file into `to`.
    fn transfer(from: &Home, to: &Home, project: &ProjectSlug, now: Timestamp) -> PathBuf {
        let file = from.root().join("export.json");
        export(from, project, &file, now).unwrap();
        import(to, project, &file, None, now).unwrap();
        file
    }

    #[test]
    fn a_derived_approval_an_import_brought_is_read_withdrawn_and_carried_on() {
        let fixture = fixture();
        let project = &fixture.project.slug;
        let exporter = Home::at_user_home(fixture._dir.path().join("exporter"));
        std::fs::create_dir_all(exporter.root()).unwrap();
        let now = at("2026-11-24T00:00:00Z");
        let candidate = sha('c');
        let mut db = _published::open(&exporter).unwrap();
        db.write(|tx| _published::record_derived(tx, project, &tick_derived(&candidate, now)))
            .unwrap();
        drop(db);
        transfer(&exporter, &fixture.home, project, now);

        let brought = derived_approval(&fixture.home, project, &candidate)
            .unwrap()
            .expect("the imported derived approval is read");
        assert!(brought.imported && brought.withdrawn.is_none());
        assert_eq!(brought.canonical, sha('b').to_string());
        assert_eq!(
            standing_derived(&fixture.home, project).unwrap(),
            std::slice::from_ref(&brought)
        );
        let again = fixture.home.root().join("again.json");
        assert_eq!(
            export(&fixture.home, project, &again, now).unwrap().derived,
            1
        );

        let mut db = _published::open(&fixture.home).unwrap();
        db.write(|tx| _published::withdraw(tx, project, &brought, &sha('e'), now))
            .unwrap();
        drop(db);
        let withdrawn = derived_approval(&fixture.home, project, &candidate)
            .unwrap()
            .unwrap();
        assert_eq!(withdrawn.withdrawn, Some((now, sha('e').to_string())));
        assert!(
            withdrawn.imported,
            "the withdrawal keeps where it came from"
        );
        assert!(standing_derived(&fixture.home, project).unwrap().is_empty());
        let last = fixture.home.root().join("last.json");
        assert_eq!(
            export(&fixture.home, project, &last, now).unwrap().derived,
            0
        );
    }

    #[test]
    fn an_export_whose_derived_approval_is_not_named_by_its_coverage_is_refused() {
        let fixture = fixture();
        let project = &fixture.project.slug;
        let now = at("2026-11-24T00:00:00Z");
        let export = _transfer::Export {
            project: project.to_string(),
            computer: "elsewhere".into(),
            written_at: now.to_string(),
            verdicts: Vec::new(),
            exemptions: Vec::new(),
            derived: vec![_transfer::ExportedDerived {
                candidate: sha('c').to_string(),
                canonical: sha('b').to_string(),
                coverage: "covered".into(),
                name: ContentDigest::of("something else").to_string(),
                moment: Moment::decided(now, None).text(),
            }],
        };
        let (text, _) = _transfer::write(&export).unwrap();
        let file = fixture.home.root().join("bad.json");
        std::fs::write(&file, text).unwrap();
        assert!(matches!(
            import(&fixture.home, project, &file, None, now),
            Err(ReviewError::Refused(_))
        ));
        assert_eq!(
            derived_approval(&fixture.home, project, &sha('c')).unwrap(),
            None
        );
    }

    #[test]
    fn a_commits_evidence_gives_its_derived_approval_and_its_withdrawal() {
        let fixture = fixture();
        let project = &fixture.project.slug;
        let now = at("2026-11-24T00:00:00Z");
        let candidate = sha('c');
        let approval = tick_derived(&candidate, now);
        let mut db = _published::open(&fixture.home).unwrap();
        db.write(|tx| _published::record_derived(tx, project, &approval))
            .unwrap();
        drop(db);
        let held = evidence(&fixture.home, project, &candidate).unwrap();
        assert_eq!(held.derived, Some(approval.clone()));
        let mut db = _published::open(&fixture.home).unwrap();
        db.write(|tx| _published::withdraw(tx, project, &approval, &sha('e'), now))
            .unwrap();
        drop(db);
        let held = evidence(&fixture.home, project, &candidate).unwrap();
        assert_eq!(
            held.derived.and_then(|d| d.withdrawn),
            Some((now, sha('e').to_string()))
        );
    }

    #[test]
    fn a_standing_derived_approval_is_the_commits_verdict_and_a_withdrawn_one_is_not() {
        let fixture = fixture();
        let project = &fixture.project.slug;
        let now = at("2026-11-24T00:00:00Z");
        let candidate = sha('c');
        assert!(
            !evidence(&fixture.home, project, &candidate)
                .unwrap()
                .carries_verdict()
        );
        let approval = tick_derived(&candidate, now);
        let mut db = _published::open(&fixture.home).unwrap();
        db.write(|tx| _published::record_derived(tx, project, &approval))
            .unwrap();
        drop(db);
        assert!(
            evidence(&fixture.home, project, &candidate)
                .unwrap()
                .carries_verdict()
        );
        let mut db = _published::open(&fixture.home).unwrap();
        db.write(|tx| _published::withdraw(tx, project, &approval, &sha('e'), now))
            .unwrap();
        drop(db);
        assert!(
            !evidence(&fixture.home, project, &candidate)
                .unwrap()
                .carries_verdict()
        );
    }

    fn exempt_side(fixture: &Fixture, commit: &CommitSha, now: Timestamp) {
        exempt(
            &fixture.home,
            &fixture.project,
            &ExemptRequest {
                commit: commit.to_string(),
                basis: "a typo fix".into(),
                caller: None,
            },
            &|| now,
        )
        .unwrap();
    }

    #[test]
    fn a_handed_off_verdict_is_history_after_the_retention_period() {
        let fixture = fixture();
        let project = &fixture.project.slug;
        let old = at("2026-10-01T00:00:00Z");
        settled(&fixture, &fixture.side, old);
        let mut db = _store::open(&fixture.home).unwrap();
        db.write(|tx| _store::hand_off(tx, project, "sha256:file", old))
            .unwrap();
        drop(db);
        let held = |fixture: &Fixture| {
            _store::held(
                &_store::reader(&fixture.home).unwrap().unwrap(),
                project,
                &fixture.side,
            )
            .unwrap()
        };
        assert!(matches!(held(&fixture), Held::HandedOff { .. }));
        git(&fixture.project.root, &["checkout", "-qb", "other"]);
        let other = commit(&fixture.project.root, "other.txt", "other");
        exempt_side(&fixture, &other, at("2026-11-24T00:00:00Z"));
        assert_eq!(
            held(&fixture),
            Held::Nothing,
            "the commit admits a verdict again"
        );
    }

    #[test]
    fn an_exemption_of_a_commit_no_branch_reaches_any_more_is_history() {
        let fixture = fixture();
        let project = &fixture.project.slug;
        exempt_side(&fixture, &fixture.side, at("2026-10-01T00:00:00Z"));
        git(&fixture.project.root, &["branch", "-qD", "side"]);
        git(&fixture.project.root, &["checkout", "-qb", "other"]);
        let other = commit(&fixture.project.root, "other.txt", "other");
        exempt_side(&fixture, &other, at("2026-11-24T00:00:00Z"));
        assert_eq!(
            evidence(&fixture.home, project, &fixture.side)
                .unwrap()
                .exemption,
            None
        );
        assert!(
            evidence(&fixture.home, project, &other)
                .unwrap()
                .exemption
                .is_some()
        );
    }

    #[test]
    fn an_exemption_of_a_commit_a_branch_still_reaches_stays_current() {
        let fixture = fixture();
        let project = &fixture.project.slug;
        exempt_side(&fixture, &fixture.side, at("2026-10-01T00:00:00Z"));
        git(&fixture.project.root, &["checkout", "-qb", "other"]);
        let other = commit(&fixture.project.root, "other.txt", "other");
        exempt_side(&fixture, &other, at("2026-11-24T00:00:00Z"));
        assert!(
            evidence(&fixture.home, project, &fixture.side)
                .unwrap()
                .exemption
                .is_some()
        );
    }

    #[test]
    fn the_tick_retires_withdrawn_and_past_derived_approvals_and_keeps_the_candidates() {
        let fixture = fixture();
        let project = &fixture.project.slug;
        let old = at("2026-10-01T00:00:00Z");
        let (withdrawn, past, active) = (sha('c'), sha('d'), sha('e'));
        let mut db = _published::open(&fixture.home).unwrap();
        db.write(|tx| {
            for candidate in [&withdrawn, &past, &active] {
                _published::record_derived(tx, project, &tick_derived(candidate, old))?;
            }
            _published::withdraw(tx, project, &tick_derived(&withdrawn, old), &sha('f'), old)?;
            _published::retire(
                tx,
                project,
                retention_cutoff(at("2026-11-24T00:00:00Z")),
                std::slice::from_ref(&active),
            )
        })
        .unwrap();
        drop(db);
        let held =
            |candidate: &CommitSha| derived_approval(&fixture.home, project, candidate).unwrap();
        assert_eq!(held(&withdrawn), None);
        assert_eq!(held(&past), None);
        assert!(held(&active).is_some());
    }

    fn build(verdict: crate::evidence::BuildVerdict, target: char) -> crate::evidence::StampFact {
        crate::evidence::StampFact::BuildVerdict {
            verdict,
            target: sha(target),
            report: ContentDigest::of("report"),
        }
    }

    #[test]
    fn a_finalized_approval_cites_the_todos_pull_request_once_its_own_verdict_is_reflected_there() {
        use crate::evidence::BuildVerdict::{Approve, ChangesRequested};
        let facts = [build(ChangesRequested, 'a'), build(Approve, 'a')];
        let stamps = [(3, &facts[0]), (7, &facts[1])];
        let approval = record_approval(stamps, Some(&sha('a'))).unwrap();
        assert_eq!(approval.build, Some(7));
        assert_eq!(approval.cited_pull(Some(&(12, vec![3, 7]))), Some(12));
        assert_eq!(
            approval.cited_pull(Some(&(12, vec![3]))),
            None,
            "not yet reflected there"
        );
        assert_eq!(approval.cited_pull(None), None);
    }

    #[test]
    fn an_approval_is_cited_when_an_earlier_verdict_is_not_reflected() {
        use crate::evidence::BuildVerdict::{Approve, ChangesRequested};
        let facts = [build(ChangesRequested, 'a'), build(Approve, 'a')];
        let stamps = [(3, &facts[0]), (7, &facts[1])];
        let approval = record_approval(stamps, Some(&sha('a'))).unwrap();
        assert_eq!(approval.cited_pull(Some(&(12, vec![7]))), Some(12));
    }

    #[test]
    fn a_reviewed_preparation_heads_approval_cites_no_pull_request() {
        let facts = [crate::evidence::StampFact::PreparationHeadReviewed {
            verdict: crate::evidence::ReviewVerdict::Approve,
            head: sha('a'),
            report: ContentDigest::of("report"),
        }];
        let approval = record_approval([(1, &facts[0])], Some(&sha('a'))).unwrap();
        assert_eq!(approval.build, None);
        assert_eq!(approval.cited_pull(Some(&(12, vec![1]))), None);
    }
}
