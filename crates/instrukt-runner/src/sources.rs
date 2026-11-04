//! Event sources (`event-activation`, "Sources and reactions"): state read
//! on each tick and translated into instrukt's event vocabulary.
//!
//! - Watched upstream refs, read with `git ls-remote` (no API quota). The
//!   first observation of a ref records its revision; a later different
//!   revision is a change.
//! - Local main against remote main, with local git and no API call.
//! - The active integrate candidate: the fetched `origin/integrate` head
//!   while remote canonical main does not contain it.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use instrukt_core::config::WatchedRef;
use instrukt_core::git::{self, GitError};
use instrukt_core::ids::{CommitSha, ProjectSlug};

use crate::events::Event;
use crate::process::{Invocation, ProcessError};
use crate::state::{self, RunnerState, StateError};

/// How long `git ls-remote` may take for one watched ref.
const LS_REMOTE_TIMEOUT: Duration = Duration::from_secs(60);

/// The remote whose branches are the project's canonical ones.
pub const CANONICAL_REMOTE: &str = "origin";

/// The branch integration candidates are pushed to.
pub const INTEGRATE_BRANCH: &str = "integrate";

/// A source could not be read.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("`git ls-remote {remote} {git_ref}` in {dir}: {detail}")]
    LsRemote {
        dir: String,
        remote: String,
        git_ref: String,
        detail: String,
    },
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    State(#[from] StateError),
}

/// The revision `git_ref` has on `remote`, as `git ls-remote` reports it;
/// `None` when the remote has no such ref.
pub fn ls_remote(
    root: &Path,
    remote: &str,
    git_ref: &str,
) -> Result<Option<CommitSha>, SourceError> {
    let done = Invocation::program("git", &["ls-remote", "--", remote, git_ref])
        .env("GIT_TERMINAL_PROMPT", "0")
        .cwd(root)
        .run(LS_REMOTE_TIMEOUT)?;
    if !done.success {
        return Err(SourceError::LsRemote {
            dir: root.display().to_string(),
            remote: remote.to_owned(),
            git_ref: git_ref.to_owned(),
            detail: done.stderr.trim().to_owned(),
        });
    }
    Ok(done
        .stdout
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(_, name)| *name == git_ref)
        .and_then(|(sha, _)| CommitSha::parse(sha).ok()))
}

/// The upstream changes of `project`'s watched refs since the last tick.
/// Each changed ref yields its event; observations are kept only after
/// `emit` accepted the events, so a failure re-reads the change next tick.
pub fn upstream_changes(
    state: &RunnerState,
    project: &ProjectSlug,
    root: &Path,
    watched: &[WatchedRef],
    mut emit: impl FnMut(&Event) -> bool,
) -> Result<(), SourceError> {
    let path = state.watched(project);
    let mut known: BTreeMap<String, CommitSha> = state::read_json(&path)?.unwrap_or_default();
    let mut changed = false;
    for watch in watched {
        let key = format!("{} {}", watch.remote, watch.git_ref);
        let observed = match ls_remote(root, &watch.remote, &watch.git_ref) {
            Ok(Some(sha)) => sha,
            Ok(None) => {
                tracing::warn!(event = "upstream_ref_absent", project = %project, remote = %watch.remote, git_ref = %watch.git_ref);
                continue;
            }
            Err(err) => {
                tracing::warn!(event = "upstream_unreadable", project = %project, remote = %watch.remote, git_ref = %watch.git_ref, error = %err);
                continue;
            }
        };
        match known.get(&key) {
            Some(previous) if previous == &observed => continue,
            Some(_) => {
                let event = Event::UpstreamChanged {
                    project: project.clone(),
                    remote: watch.remote.clone(),
                    r#ref: watch.git_ref.clone(),
                    revision: observed.clone(),
                };
                if !emit(&event) {
                    continue;
                }
            }
            None => {
                tracing::info!(event = "upstream_ref_first_observed", project = %project, remote = %watch.remote, git_ref = %watch.git_ref, revision = %observed);
            }
        }
        known.insert(key, observed);
        changed = true;
    }
    if changed {
        state::write_json(&path, &known)?;
    }
    Ok(())
}

/// What local git says about a project's main and its candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainObservation {
    pub local_main: Option<CommitSha>,
    pub remote_main: Option<CommitSha>,
    /// Local main strictly ahead of remote main.
    pub ahead: bool,
    /// The active integrate candidate.
    pub candidate: Option<CommitSha>,
}

impl MainObservation {
    /// `runner.main_ahead` when local main is ahead and no candidate is
    /// pending.
    pub fn main_ahead(&self, project: &ProjectSlug) -> Option<Event> {
        match (
            &self.local_main,
            &self.remote_main,
            self.ahead,
            &self.candidate,
        ) {
            (Some(local), Some(remote), true, None) => Some(Event::MainAhead {
                project: project.clone(),
                local_main: local.clone(),
                remote_main: remote.clone(),
            }),
            _ => None,
        }
    }
}

/// Reads local main, and the remote's current canonical main and
/// `integrate` heads, and the active candidate. The remote heads are the
/// remote's own answer (`git ls-remote`), never a tracking ref a fetch may
/// not have refreshed; the commits are then fetched into their tracking
/// refs so ancestry can be judged against them.
pub fn observe_main(root: &Path) -> Result<MainObservation, SourceError> {
    let branch = git::default_branch(root, CANONICAL_REMOTE)?;
    let local_main = resolve(root, &format!("refs/heads/{branch}"))?;
    let (remote_main, integrate) = remote_heads(root, &branch)?;
    let ahead = match (&local_main, &remote_main) {
        (Some(local), Some(remote)) if local != remote => git::succeeds(
            root,
            [
                "merge-base",
                "--is-ancestor",
                remote.as_str(),
                local.as_str(),
            ],
        )?,
        _ => false,
    };
    let candidate = match (integrate, &remote_main) {
        (Some(head), Some(main)) => {
            let promoted = git::succeeds(
                root,
                ["merge-base", "--is-ancestor", head.as_str(), main.as_str()],
            )?;
            (!promoted).then_some(head)
        }
        (Some(head), None) => Some(head),
        (None, _) => None,
    };
    Ok(MainObservation {
        local_main,
        remote_main,
        ahead,
        candidate,
    })
}

/// How many times a sample of the remote's heads is taken before the
/// tick gives the observation up.
const SAMPLE_ATTEMPTS: u32 = 3;

/// What follows a sampling attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sampling {
    /// The sampled commits are available locally; judge them.
    Observed,
    /// A sampled commit could not be fetched (the ref moved on); sample
    /// again.
    Resample,
    /// The bound is spent; this tick's observation failed and the next
    /// tick retries.
    GiveUp,
}

/// The step after attempt `attempt` (from 1) whose sampled commits were
/// `fetched` or not.
pub fn after_sample(attempt: u32, fetched: bool) -> Sampling {
    match (fetched, attempt < SAMPLE_ATTEMPTS) {
        (true, _) => Sampling::Observed,
        (false, true) => Sampling::Resample,
        (false, false) => Sampling::GiveUp,
    }
}

/// The canonical remote's current heads of `branch` and `integrate`, as
/// it answers now; `(None, None)` when the project has no canonical remote.
/// The sampled commits themselves — never whatever the branch names by the
/// time of a later fetch — are fetched and recorded in the tracking refs,
/// so the commits judged are exactly the commits observed. A sampled
/// commit that is no longer available is sampled afresh, a bounded number
/// of times.
fn remote_heads(
    root: &Path,
    branch: &str,
) -> Result<(Option<CommitSha>, Option<CommitSha>), SourceError> {
    let remotes = git::run(root, ["remote"])?;
    if !remotes
        .lines()
        .any(|remote| remote.trim() == CANONICAL_REMOTE)
    {
        return Ok((None, None));
    }
    let mut attempt = 1;
    loop {
        let (main, integrate) = sample(root, branch)?;
        let sampled: Vec<&CommitSha> = main.iter().chain(integrate.iter()).collect();
        let fetched = fetch_commits(root, &sampled)?;
        match after_sample(attempt, fetched) {
            Sampling::Observed => {
                for (sha, name) in [(&main, branch), (&integrate, INTEGRATE_BRANCH)] {
                    if let Some(sha) = sha {
                        git::run(
                            root,
                            [
                                "update-ref",
                                &format!("refs/remotes/{CANONICAL_REMOTE}/{name}"),
                                sha.as_str(),
                            ],
                        )?;
                    }
                }
                return Ok((main, integrate));
            }
            Sampling::Resample => {
                tracing::debug!(event = "remote_heads_resampled", attempt);
                attempt += 1;
            }
            Sampling::GiveUp => {
                tracing::warn!(event = "remote_heads_unstable", attempts = attempt, dir = %root.display());
                return Err(SourceError::LsRemote {
                    dir: root.display().to_string(),
                    remote: CANONICAL_REMOTE.to_owned(),
                    git_ref: format!("refs/heads/{branch} refs/heads/{INTEGRATE_BRANCH}"),
                    detail: format!(
                        "the sampled heads kept moving before they could be fetched ({attempt} attempts); the next tick observes again"
                    ),
                });
            }
        }
    }
}

/// One `git ls-remote` sample of the canonical branch and `integrate`.
fn sample(
    root: &Path,
    branch: &str,
) -> Result<(Option<CommitSha>, Option<CommitSha>), SourceError> {
    let main_ref = format!("refs/heads/{branch}");
    let integrate_ref = format!("refs/heads/{INTEGRATE_BRANCH}");
    let done = Invocation::program(
        "git",
        &[
            "ls-remote",
            "--",
            CANONICAL_REMOTE,
            &main_ref,
            &integrate_ref,
        ],
    )
    .env("GIT_TERMINAL_PROMPT", "0")
    .cwd(root)
    .run(LS_REMOTE_TIMEOUT)?;
    if !done.success {
        return Err(SourceError::LsRemote {
            dir: root.display().to_string(),
            remote: CANONICAL_REMOTE.to_owned(),
            git_ref: format!("{main_ref} {integrate_ref}"),
            detail: done.stderr.trim().to_owned(),
        });
    }
    let head = |name: &str| {
        done.stdout
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .find(|(_, listed)| *listed == name)
            .and_then(|(sha, _)| CommitSha::parse(sha).ok())
    };
    Ok((head(&main_ref), head(&integrate_ref)))
}

/// Makes `commits` available locally, fetching by object id those not
/// present yet; `false` when the remote no longer serves one of them.
fn fetch_commits(root: &Path, commits: &[&CommitSha]) -> Result<bool, SourceError> {
    let mut missing = Vec::new();
    for commit in commits {
        // `cat-file -e` on a full id answers presence without resolving a
        // name: any failure means the object is not here (yet).
        let present = git::run_output(root, ["cat-file", "-e", commit.as_str()])?
            .status
            .success();
        if !present {
            missing.push(commit.as_str());
        }
    }
    if missing.is_empty() {
        return Ok(true);
    }
    let mut args = vec![
        "fetch",
        "--quiet",
        "--no-tags",
        "--no-write-fetch-head",
        "--",
        CANONICAL_REMOTE,
    ];
    args.extend(missing.iter().copied());
    let fetched = Invocation::program("git", &args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .cwd(root)
        .run(LS_REMOTE_TIMEOUT)?;
    if !fetched.success {
        tracing::debug!(event = "sampled_commit_unfetchable", stderr = %fetched.stderr.trim());
    }
    Ok(fetched.success)
}

/// The commit `git_ref` names; `None` when the ref does not exist.
fn resolve(root: &Path, git_ref: &str) -> Result<Option<CommitSha>, SourceError> {
    let output = git::run_output(
        root,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{git_ref}^{{commit}}"),
        ],
    )?;
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    if !output.status.success() {
        return Err(SourceError::Git(git::GitError::Failed {
            dir: root.to_path_buf(),
            args: format!("rev-parse --verify --quiet {git_ref}"),
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        }));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(CommitSha::parse(text.trim()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(c: char) -> CommitSha {
        CommitSha::parse(&c.to_string().repeat(40)).unwrap()
    }

    #[test]
    fn main_ahead_needs_local_ahead_and_no_pending_candidate() {
        let project = ProjectSlug::parse("demo").unwrap();
        let ahead = MainObservation {
            local_main: Some(sha('a')),
            remote_main: Some(sha('b')),
            ahead: true,
            candidate: None,
        };
        assert_eq!(
            ahead.main_ahead(&project),
            Some(Event::MainAhead {
                project: project.clone(),
                local_main: sha('a'),
                remote_main: sha('b'),
            })
        );
        let pending = MainObservation {
            candidate: Some(sha('c')),
            ..ahead.clone()
        };
        assert_eq!(pending.main_ahead(&project), None);
        let level = MainObservation {
            ahead: false,
            ..ahead
        };
        assert_eq!(level.main_ahead(&project), None);
    }

    #[test]
    fn a_sample_that_cannot_be_fetched_is_retaken_then_given_up_at_the_bound() {
        assert_eq!(after_sample(1, true), Sampling::Observed);
        assert_eq!(after_sample(1, false), Sampling::Resample);
        assert_eq!(after_sample(SAMPLE_ATTEMPTS - 1, false), Sampling::Resample);
        assert_eq!(after_sample(SAMPLE_ATTEMPTS, false), Sampling::GiveUp);
        assert_eq!(after_sample(SAMPLE_ATTEMPTS, true), Sampling::Observed);
    }
}
