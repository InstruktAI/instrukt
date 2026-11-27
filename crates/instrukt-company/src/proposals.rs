//! Knowledge proposals (`project/instrukt/spec/domain/company`, Knowledge
//! proposal): which commits a company's conversation sessions make in the
//! company's repository were asked for by an administrator of the company,
//! recorded once each commit is made, and whether a head is an admitted
//! proposal, which the runner carries to review and integration with no
//! person moving it.
//!
//! The recording reads the speaker of the committing session's turn in
//! progress through [`crate::conversations::speaker`], the lookup every
//! speaker decision of this context uses. Nothing the agent writes makes a
//! commit a proposal. Admission compares recorded facts and paths; it judges
//! no text.

mod _store;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use instrukt_core::config::{CONFIG_FILE_NAME, CompanyIdentity, Configuration, Env, Runtime};
use instrukt_core::git;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use instrukt_runner::interfaces::{CompanyError, ProposalStoreCondition, UnestablishedProposal};
use instrukt_runner::mcp::{McpError, Speaker};
use instrukt_runner::runtime::RuntimeAdapter;
use jiff::Timestamp;

use _store::Recorded;
use _store::{Asked, Entry};

/// How long the recording asks the speaker lookup again after an error
/// before it records the commit unestablished.
const SPEAKER_RETRY: Duration = Duration::from_secs(5);

/// The pause between two speaker lookups.
const SPEAKER_PAUSE: Duration = Duration::from_millis(250);

/// The remote whose tracking branches a proposal's head and the canonical
/// branch are read from.
const REMOTE: &str = "origin";

/// What the speaker lookup's answer makes of a commit in the repository of
/// the company `company`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SpeakerDecision {
    /// No proposal: no speaker, another company's, or no administrator.
    NotAProposal,
    /// An administrator of the company asked for it.
    Proposal(Speaker),
    /// The lookup could not answer: why. It is asked again.
    Retry(String),
}

/// Decides what `answer`, the speaker lookup's, makes of a commit in the
/// repository of `company`. An administrator is admitted in a direct
/// message and a channel alike. An error — a session the runner has not
/// recorded yet, the state database or the runtime's turn — is retried.
fn decide_speaker(
    answer: Result<Option<Speaker>, McpError>,
    company: &ProjectSlug,
) -> SpeakerDecision {
    match answer {
        Ok(None) => SpeakerDecision::NotAProposal,
        Ok(Some(speaker)) if speaker.company != *company || !speaker.is_admin => {
            SpeakerDecision::NotAProposal
        }
        Ok(Some(speaker)) => SpeakerDecision::Proposal(speaker),
        Err(error) => SpeakerDecision::Retry(error.to_string()),
    }
}

/// Whether a head is an admitted proposal, and if not, why.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Admission {
    /// Every commit it adds is a proposal and every path it changes is
    /// company knowledge.
    Admitted,
    /// It adds no commit to the canonical branch.
    NothingToAdmit,
    /// This commit is no recorded proposal.
    NotAProposal(String),
    /// This path is no company knowledge.
    OutsideKnowledge(PathBuf),
}

/// Decides whether a head adding `commits` to the canonical branch and
/// changing `paths` is admitted, given what the store recorded.
fn admission(
    commits: &[String],
    recorded: &BTreeMap<String, Recorded>,
    paths: &[PathBuf],
) -> Admission {
    if commits.is_empty() {
        return Admission::NothingToAdmit;
    }
    if let Some(commit) = commits
        .iter()
        .find(|commit| recorded.get(*commit) != Some(&Recorded::Proposal))
    {
        return Admission::NotAProposal(commit.clone());
    }
    match paths
        .iter()
        .find(|path| !instrukt_core::publication::is_company_knowledge(path))
    {
        Some(path) => Admission::OutsideKnowledge(path.clone()),
        None => Admission::Admitted,
    }
}

/// What the repository says of one recorded commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Holding {
    /// The remote-tracking canonical branch contains it.
    pub contained: bool,
    /// A local branch, a remote-tracking branch or a worktree's `HEAD`
    /// holds it.
    pub held: bool,
}

/// The recorded commits a recording retires: each the remote-tracking
/// canonical branch contains, or nothing holds any more.
fn retired(facts: &[(String, Holding)]) -> Vec<String> {
    facts
        .iter()
        .filter(|(_, holding)| holding.contained || !holding.held)
        .map(|(commit, _)| commit.clone())
        .collect()
}

/// The company's recording of each commit made in its repository, which
/// the `post-commit` hook run invokes. It decides everything and never
/// fails the commit.
pub struct Recording {
    clock: Arc<dyn Fn() -> Timestamp + Send + Sync>,
    adapters: Arc<Adapters>,
}

/// The runtime adapter for the runtime the machine's configuration selects,
/// as the composition root supplies it.
pub type Adapters = dyn Fn(Runtime) -> Box<dyn RuntimeAdapter> + Send + Sync;

impl Recording {
    /// A recording that stamps each entry with `clock`'s time and asks the
    /// committing turn's speaker through the adapter `adapters` supplies for
    /// the selected runtime.
    pub fn new(clock: Arc<dyn Fn() -> Timestamp + Send + Sync>, adapters: Arc<Adapters>) -> Self {
        Self { clock, adapters }
    }

    /// Records the commit just made at `root` as a knowledge proposal when
    /// an administrator of the repository's company asked for it in the
    /// committing session's turn, or as unestablished when the speaker
    /// cannot be established. `env` is the hook's environment, which the
    /// committing session's is. A failure is reported to the log and on
    /// `stderr`, the commit's standard error, and leaves the commit
    /// unrecorded, waiting for a person.
    pub fn record(&self, home: &Home, root: &Path, env: &Env, stderr: &mut dyn Write) {
        if let Err(cause) = self.try_record(home, root, env, stderr) {
            report(
                stderr,
                "knowledge_proposal_not_recorded",
                &format!(
                    "the commit is not recorded as a knowledge proposal and waits for a person: {cause}"
                ),
            );
        }
    }

    fn try_record(
        &self,
        home: &Home,
        root: &Path,
        env: &Env,
        stderr: &mut dyn Write,
    ) -> Result<(), String> {
        if !instrukt_core::publication::is_company_repository(root).map_err(|e| e.to_string())? {
            return Ok(());
        }
        let inspection = home.for_inspection();
        let global = Configuration::load_global(&inspection).map_err(|e| e.to_string())?;
        let Some(session) = global.caller(env).map_err(|e| e.to_string())? else {
            return Ok(());
        };
        let identity = identity(root)?;
        let adapter = (self.adapters)(global.runner_runtime().map_err(|e| e.to_string())?);
        let first = Instant::now();
        let asked = loop {
            let answer = crate::conversations::speaker(&inspection, &session, adapter.as_ref());
            match decide_speaker(answer, &identity.key) {
                SpeakerDecision::NotAProposal => return Ok(()),
                SpeakerDecision::Proposal(speaker) => break (Some(speaker), None),
                SpeakerDecision::Retry(cause)
                    if first.elapsed() + SPEAKER_PAUSE < SPEAKER_RETRY =>
                {
                    tracing::warn!(
                        event = "knowledge_proposal_speaker_retry",
                        session = %session,
                        cause = %cause,
                        "the speaker of the committing turn could not be established; asking again"
                    );
                    std::thread::sleep(SPEAKER_PAUSE);
                }
                SpeakerDecision::Retry(cause) => break (None, Some(cause)),
            }
        };
        let commit = head(root)?;
        let entry = match asked {
            (Some(speaker), _) => Entry {
                company: identity.key.clone(),
                commit,
                branch: branch(root),
                connector: speaker.connector.clone(),
                conversation: speaker.conversation.clone(),
                session: session.to_string(),
                recorded_at: (self.clock)(),
                asked: Asked::Administrator {
                    identity: speaker.identity.clone(),
                    name: speaker.name.clone(),
                    is_direct: speaker.is_direct,
                },
            },
            (None, cause) => {
                let cause = cause.unwrap_or_default();
                report(
                    stderr,
                    "knowledge_proposal_unestablished",
                    &format!(
                        "the speaker of the committing turn could not be established, so the commit is recorded unestablished and waits for a person: {cause}"
                    ),
                );
                Entry {
                    company: identity.key.clone(),
                    commit,
                    branch: branch(root),
                    connector: None,
                    conversation: None,
                    session: session.to_string(),
                    recorded_at: (self.clock)(),
                    asked: Asked::Unestablished { cause },
                }
            }
        };
        // The retirement is computed before the store's lock is taken; a git
        // read that fails skips it for this recording, and the next retires.
        let retire = match retirement(home, &identity.key, root) {
            Ok(retire) => retire,
            Err(cause) => {
                report(
                    stderr,
                    "knowledge_proposal_retirement_skipped",
                    &format!("no recorded proposal was retired this time: {cause}"),
                );
                Vec::new()
            }
        };
        _store::record(home, &entry, &retire).map_err(|e| e.to_string())
    }
}

impl instrukt_core::interfaces::ProposalRecording for Recording {
    fn record(&self, home: &Home, root: &Path, env: &Env, stderr: &mut dyn Write) {
        Recording::record(self, home, root, env, stderr);
    }
}

/// Reports `message` to the log under `event` and as one line on the
/// commit's standard error.
fn report(stderr: &mut dyn Write, event: &'static str, message: &str) {
    tracing::warn!(event, "{message}");
    if let Err(error) = writeln!(stderr, "instrukt: {message}") {
        tracing::debug!(event = "knowledge_proposal_report_unwritten", error = %error);
    }
}

/// The company identity the repository at `root` declares in its project
/// layer.
fn identity(root: &Path) -> Result<CompanyIdentity, String> {
    let path = root.join(CONFIG_FILE_NAME);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    instrukt_core::config::company_identity(bytes.as_deref()).map_err(|e| e.to_string())
}

fn head(root: &Path) -> Result<CommitSha, String> {
    let text = git::run(root, ["rev-parse", "HEAD"]).map_err(|e| e.to_string())?;
    CommitSha::parse(text.trim()).map_err(|e| e.to_string())
}

/// The branch the commit was made on; `None` on a detached `HEAD`.
fn branch(root: &Path) -> Option<String> {
    git::run(root, ["branch", "--show-current"])
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// The remote-tracking canonical branch, when the repository has fetched
/// one.
fn canonical(root: &Path) -> Result<Option<String>, String> {
    let branch = git::default_branch(root, REMOTE).map_err(|e| e.to_string())?;
    let reference = format!("refs/remotes/{REMOTE}/{branch}");
    let exists = git::succeeds(
        root,
        ["show-ref", "--verify", "--quiet", reference.as_str()],
    )
    .map_err(|e| e.to_string())?;
    Ok(exists.then_some(reference))
}

/// Whether `commit` is an object the repository holds.
fn present(root: &Path, commit: &str) -> Result<bool, String> {
    git::answers(root, ["cat-file", "-e", &format!("{commit}^{{commit}}")])
        .map_err(|e| e.to_string())
}

fn contains(root: &Path, commit: &str, descendant: &str) -> Result<bool, String> {
    git::answers(root, ["merge-base", "--is-ancestor", commit, descendant])
        .map_err(|e| e.to_string())
}

/// Every worktree's `HEAD` commit.
fn worktree_heads(root: &Path) -> Result<Vec<String>, String> {
    let listing = git::run(root, ["worktree", "list", "--porcelain"]).map_err(|e| e.to_string())?;
    Ok(listing
        .lines()
        .filter_map(|line| line.strip_prefix("HEAD "))
        .map(|sha| sha.trim().to_owned())
        .collect())
}

/// What the repository at `root` says of `commit`.
fn holding(
    root: &Path,
    commit: &str,
    canonical: Option<&str>,
    heads: &[String],
) -> Result<Holding, String> {
    if !present(root, commit)? {
        return Ok(Holding {
            contained: false,
            held: false,
        });
    }
    let contained = match canonical {
        Some(canonical) => contains(root, commit, canonical)?,
        None => false,
    };
    let refs = git::run(
        root,
        [
            "for-each-ref",
            "--count=1",
            "--format=%(refname)",
            "--contains",
            commit,
            "refs/heads",
            "refs/remotes",
        ],
    )
    .map_err(|e| e.to_string())?;
    let mut held = !refs.trim().is_empty();
    for head in heads {
        if held {
            break;
        }
        held = contains(root, commit, head)?;
    }
    Ok(Holding { contained, held })
}

/// The recorded commits of `company` a recording now retires.
fn retirement(home: &Home, company: &ProjectSlug, root: &Path) -> Result<Vec<String>, String> {
    let recorded = _store::read(home, company).map_err(|e| e.to_string())?;
    if recorded.is_empty() {
        return Ok(Vec::new());
    }
    let canonical = canonical(root)?;
    let heads = worktree_heads(root)?;
    let mut facts = Vec::new();
    for commit in recorded.keys() {
        facts.push((
            commit.clone(),
            holding(root, commit, canonical.as_deref(), &heads)?,
        ));
    }
    Ok(retired(&facts))
}

fn lookup_error(error: impl std::fmt::Display) -> CompanyError {
    CompanyError(error.to_string())
}

/// Whether `head` is an admitted proposal of the company repository at
/// `root`, against its remote-tracking canonical branch.
pub(crate) fn admitted(home: &Home, root: &Path, head: &CommitSha) -> Result<bool, CompanyError> {
    let identity = identity(root).map_err(lookup_error)?;
    let Some(canonical) = canonical(root).map_err(lookup_error)? else {
        return Ok(false);
    };
    let range = format!("{canonical}..{head}");
    let commits: Vec<String> = git::run(root, ["rev-list", range.as_str()])
        .map_err(lookup_error)?
        .lines()
        .map(str::to_owned)
        .collect();
    let changed = format!("{canonical}...{head}");
    let paths: Vec<PathBuf> = git::run(
        root,
        ["diff", "--name-only", "--no-renames", changed.as_str()],
    )
    .map_err(lookup_error)?
    .lines()
    .map(PathBuf::from)
    .collect();
    let recorded = _store::read(home, &identity.key).map_err(lookup_error)?;
    Ok(admission(&commits, &recorded, &paths) == Admission::Admitted)
}

/// The unestablished entries of the company repository at `root` whose
/// commit a remote-tracking branch holds.
pub(crate) fn unestablished(
    home: &Home,
    root: &Path,
) -> Result<Vec<UnestablishedProposal>, CompanyError> {
    let identity = identity(root).map_err(lookup_error)?;
    let recorded = _store::read(home, &identity.key).map_err(lookup_error)?;
    let mut found = Vec::new();
    for (commit, recorded) in recorded {
        let Recorded::Unestablished(cause) = recorded else {
            continue;
        };
        if !present(root, &commit).map_err(lookup_error)? {
            continue;
        }
        let remotes = git::run(
            root,
            [
                "for-each-ref",
                "--count=1",
                "--format=%(refname)",
                "--contains",
                commit.as_str(),
                "refs/remotes",
            ],
        )
        .map_err(lookup_error)?;
        if remotes.trim().is_empty() {
            continue;
        }
        found.push(UnestablishedProposal {
            company: identity.key.clone(),
            commit: CommitSha::parse(&commit).map_err(lookup_error)?,
            cause,
        });
    }
    Ok(found)
}

/// The condition of the proposal store of the company repository at
/// `root`: its stored version read without migrating it, and whether a
/// writer could write it.
pub(crate) fn condition(home: &Home, root: &Path) -> Result<ProposalStoreCondition, CompanyError> {
    let identity = identity(root).map_err(lookup_error)?;
    let store = _store::store(home, &identity.key);
    Ok(match store.stored_schema() {
        Err(error) => ProposalStoreCondition::Unreadable(error.to_string()),
        Ok(Some(found)) if found > store.version() => ProposalStoreCondition::UnknownShape,
        Ok(_) if !store.writable() => ProposalStoreCondition::Unwritable,
        Ok(Some(found)) if found < store.version() => ProposalStoreCondition::Older,
        Ok(_) => ProposalStoreCondition::Usable,
    })
}
