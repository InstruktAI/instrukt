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

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use instrukt_core::git;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef};
use instrukt_runner::state::RunnerState;
use jiff::Timestamp;

pub use _moment::Moment;
pub use _publication::Published;
pub use _published::{Delivery, DeliveryState, Destination, Recognised};

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
}

impl ReviewWork {
    pub fn new(home: Home, access: std::sync::Arc<super::GitHubAccess>) -> Self {
        Self { home, access }
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
        };
        publication.deliver(commit, &target, &pulls_at(commit))?;
    }
    let mut current: Vec<CommitSha> = standing.iter().map(|(commit, _)| commit.clone()).collect();
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
        };
        publication.deliver(commit, &target, &pulls)?;
    }
    let mut db = _published::open(home)?;
    db.write(|tx| _published::retire(tx, project, retention_cutoff(now), &current))
}
