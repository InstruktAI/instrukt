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
    pub deliveries: Vec<Delivery>,
    pub recognised: Option<Recognised>,
}

impl CommitEvidence {
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
        let object = format!("{sha}^{{commit}}");
        if !git::succeeds(&self.root, ["cat-file", "-e", object.as_str()])? {
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
    let answers = match _admission::request_check(&requests, &reviewer) {
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
    let held = match _store::reader(home)? {
        Some(conn) => _store::held(&conn, &project.slug, &head)?,
        None => Held::Nothing,
    };
    let returned_base = match &entering {
        Some(entering) => handed_over(home, env, &head, &reviewer, entering, request, &held)?,
        None => false,
    };
    match _admission::settlement(&held, &reviewer, &requests) {
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
    let contained = aged_contained(home, project, clock())?;
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
        _store::retire(tx, retention_cutoff(now), &contained)?;
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

/// The commits of `project` whose evidence aged past the retention period
/// and that the canonical branch contains.
fn aged_contained(
    home: &Home,
    project: &ReviewProject,
    now: Timestamp,
) -> Result<Vec<(ProjectSlug, CommitSha)>, ReviewError> {
    let Some(conn) = _store::reader(home)? else {
        return Ok(Vec::new());
    };
    let canonical = project.canonical()?;
    let mut contained = Vec::new();
    for commit in _store::aged_commits(&conn, &project.slug, retention_cutoff(now))? {
        if project.contains(commit.as_str(), &canonical)? {
            contained.push((project.slug.clone(), commit));
        }
    }
    Ok(contained)
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
    let contained = aged_contained(home, project, clock())?;
    let mut db = _store::open(home)?;
    db.write(|tx| {
        if let Some(held) = _store::exemption(tx, &project.slug, &commit)? {
            return Err(refused(format!(
                "{commit} is already exempted ({}); see `instrukt review show {commit}`",
                held.basis
            )));
        }
        _store::exempt(tx, &project.slug, &exemption)?;
        _store::retire(tx, retention_cutoff(exemption.recorded_at), &contained)?;
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
    Ok(evidence)
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
        Ok(evidence.current().is_some() || evidence.recognised.is_some())
    }

    fn candidate(
        &self,
        project: &instrukt_runner::interfaces::ProjectRef,
        repository: &str,
        candidate: &CommitSha,
        pulls: &[u64],
        now: Timestamp,
    ) -> Result<instrukt_runner::interfaces::CandidateFinding, String> {
        candidate_finding(self, project, repository, candidate, pulls, now)
            .map_err(|error| error.to_string())
    }

    fn publish(
        &self,
        project: &ProjectSlug,
        repository: &str,
        eligible: &[(u64, CommitSha)],
        now: Timestamp,
    ) -> Result<(), String> {
        publish(&self.home, &self.access, project, repository, eligible, now)
            .map_err(|error| error.to_string())
    }
}

/// Publishes the current verdict of every commit whose head verdict stands
/// here, and recognises what the reviewer bot already published for each
/// eligible head this computer holds no evidence of (`github-apps`, Verdict
/// publication). The tick's alone.
fn publish(
    home: &Home,
    access: &super::GitHubAccess,
    project: &ProjectSlug,
    repository: &str,
    eligible: &[(u64, CommitSha)],
    now: Timestamp,
) -> Result<(), ReviewError> {
    let gh = access
        .review_client(repository, now)
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
    let derived = match _published::reader(home)? {
        Some(conn) => _published::standing_derived(&conn, project)?,
        None => Vec::new(),
    };
    for approval in &derived {
        current.push(approval.candidate.clone());
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
        if standing.iter().any(|(held, _)| held == commit) {
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
    let derived = match _published::reader(home)? {
        Some(conn) => _published::derived(&conn, project)?,
        None => Vec::new(),
    };
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
        for approval in &export.derived {
            _store::import_approval(tx, project, approval, now)?;
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

/// The head a todo's record approves last by build review, when its last
/// build verdict for that head approves.
fn record_approval(
    record: &crate::evidence::TodoEvidence,
    commit: Option<&CommitSha>,
) -> Option<(CommitSha, String)> {
    use crate::evidence::{BuildVerdict, ReviewVerdict, StampFact};
    let mut last: Option<(CommitSha, String, bool)> = None;
    for stamp in &record.stamps {
        let (head, report, approves) = match &stamp.fact {
            StampFact::BuildVerdict {
                verdict,
                target,
                report,
            } => (target, report, *verdict == BuildVerdict::Approve),
            StampFact::PreparationHeadReviewed {
                verdict,
                head,
                report,
            } => (head, report, *verdict == ReviewVerdict::Approve),
            _ => continue,
        };
        if commit.is_none_or(|commit| commit == head) {
            last = Some((head.clone(), report.to_string(), approves));
        }
    }
    last.filter(|(_, _, approves)| *approves)
        .map(|(head, report, _)| {
            let reference = format!(
                "review approval of {head} in todo {} ({report})",
                record.slug
            );
            (head, reference)
        })
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
        Ok(self.todo_of(commit)?.and_then(|record| {
            record_approval(&record, Some(commit)).map(|(_, reference)| crate::workflow::Approved {
                reference,
                todo: Some(record.slug.clone()),
            })
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
            .and_then(|record| record_approval(&record, None))
            .map(|(head, _)| head))
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
        let derived = match _published::reader(&self.work.home).map_err(|e| e.to_string())? {
            Some(conn) => _published::derived_of(&conn, &self.project.slug, head)
                .map_err(|e| e.to_string())?,
            None => None,
        };
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
) -> Result<instrukt_runner::interfaces::CandidateFinding, ReviewError> {
    use instrukt_runner::interfaces::CandidateFinding;
    let held = evidence(&work.home, &project.slug, candidate)?;
    if held
        .current()
        .is_some_and(|version| version.base_on_canonical)
    {
        return Ok(CandidateFinding::Settled);
    }
    let recorded = match _published::reader(&work.home)? {
        Some(conn) => _published::derived_of(&conn, &project.slug, candidate)?,
        None => None,
    };
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
            if recorded.as_ref().is_some_and(|a| a.withdrawn.is_none()) {
                let mut db = _published::open(&work.home)?;
                db.write(|tx| _published::withdraw(tx, &project.slug, candidate, &commit, now))?;
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
            };
            let mut db = _published::open(&work.home)?;
            db.write(|tx| _published::record_derived(tx, &project.slug, &approval))?;
            drop(db);
            tracing::info!(event = "review_derived_approval_recorded", project = %project.slug, candidate = %candidate, items = items.len());
            let gh = work
                .access
                .review_client(repository, now)
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
