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

use crate::event_log::DbEventLog;
use crate::events::Event;
use crate::events::{EventLog, EventLogError};
use crate::process::{Invocation, ProcessError};
use crate::state::{self, RunnerState, StateError};
use instrukt_core::state_db::rusqlite::params;
use jiff::Timestamp;

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
    #[error("cannot synchronize canonical checkout {dir}: {reason}")]
    Sync { dir: String, reason: String },
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Log(#[from] EventLogError),
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
/// Each changed ref yields its event; the events and the observations
/// commit together in one transaction, so a failure re-reads the change
/// next tick.
pub fn upstream_changes(
    state: &RunnerState,
    log: &DbEventLog,
    project: &ProjectSlug,
    root: &Path,
    watched: &[WatchedRef],
    now: Timestamp,
) -> Result<(), SourceError> {
    let known = state
        .read(|conn, path| {
            let sql = state::sql(path);
            let mut statement = conn
                .prepare("SELECT remote, ref, revision FROM watched_refs WHERE project = ?1")
                .map_err(&sql)?;
            let rows = statement
                .query_map(params![project.as_str()], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(&sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(&sql)?;
            rows.into_iter()
                .map(|(remote, git_ref, revision)| {
                    let sha = CommitSha::parse(&revision).map_err(|e| state::invalid(path, e))?;
                    Ok((format!("{remote} {git_ref}"), sha))
                })
                .collect::<Result<BTreeMap<_, _>, StateError>>()
        })?
        .unwrap_or_default();
    let mut observations: Vec<(&WatchedRef, CommitSha, Option<Event>)> = Vec::new();
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
                observations.push((watch, observed, Some(event)));
            }
            None => {
                tracing::info!(event = "upstream_ref_first_observed", project = %project, remote = %watch.remote, git_ref = %watch.git_ref, revision = %observed);
                observations.push((watch, observed, None));
            }
        }
    }
    if observations.is_empty() {
        return Ok(());
    }
    state.write(|tx| {
        let path = state::db_path(tx);
        for (watch, observed, event) in &observations {
            if let Some(event) = event {
                log.append(tx, event)?;
            }
            tx.execute(
                "INSERT INTO watched_refs (project, remote, ref, revision, observed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (project, remote, ref) DO UPDATE SET revision = excluded.revision,
                    observed_at = excluded.observed_at",
                params![
                    project.as_str(),
                    watch.remote,
                    watch.git_ref,
                    observed.as_str(),
                    now.to_string()
                ],
            )
            .map_err(state::sql(&path))?;
        }
        Ok::<_, SourceError>(())
    })
}

/// What local git says about a project's main and its candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainObservation {
    pub remote_main: Option<CommitSha>,
    /// The active integrate candidate.
    pub candidate: Option<CommitSha>,
}

/// Reads the remote's current canonical main and
/// `integrate` heads, and the active candidate. The remote heads are the
/// remote's own answer (`git ls-remote`), never a tracking ref a fetch may
/// not have refreshed; the commits are then fetched into their tracking
/// refs so ancestry can be judged against them.
pub fn observe_main(root: &Path) -> Result<MainObservation, SourceError> {
    let branch = git::default_branch(root, CANONICAL_REMOTE)?;
    let (remote_main, integrate) = remote_heads(root, &branch)?;
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
        remote_main,
        candidate,
    })
}

/// Advance the canonical checkout mechanically; Git's post-merge hook publishes
/// the new docs and hands its revision to blueprint. Never switch or force a checkout.
pub fn synchronize_main(root: &Path, observed: &MainObservation) -> Result<(), SourceError> {
    let Some(remote) = &observed.remote_main else {
        return Ok(());
    };
    let branch = git::default_branch(root, CANONICAL_REMOTE)?;
    let local = git::run(
        root,
        ["rev-parse", "--verify", &format!("refs/heads/{branch}")],
    )?;
    if local == remote.as_str() {
        return Ok(());
    }
    let refused = |reason: &str| SourceError::Sync {
        dir: root.display().to_string(),
        reason: reason.to_owned(),
    };
    if !git::succeeds(
        root,
        ["merge-base", "--is-ancestor", &local, remote.as_str()],
    )? {
        return Err(refused(
            "local main is not an ancestor of remote main; preserve its history and reconcile in a reviewed worktree",
        ));
    }
    let checked_out = git::run(root, ["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    if checked_out != branch {
        return Err(refused(
            "project checkout is not on canonical main; restore its intended placement before retrying",
        ));
    }
    if !git::run(root, ["status", "--porcelain=v1", "--untracked-files=all"])?.is_empty() {
        return Err(refused(
            "project checkout is dirty; preserve its changes and retry after their owner clears the checkout",
        ));
    }
    git::run(
        root,
        [
            "merge",
            "--ff-only",
            "--no-overwrite-ignore",
            remote.as_str(),
        ],
    )?;
    tracing::info!(event = "canonical_checkout_advanced", dir = %root.display(), commit = %remote);
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sample_that_cannot_be_fetched_is_retaken_then_given_up_at_the_bound() {
        assert_eq!(after_sample(1, true), Sampling::Observed);
        assert_eq!(after_sample(1, false), Sampling::Resample);
        assert_eq!(after_sample(SAMPLE_ATTEMPTS - 1, false), Sampling::Resample);
        assert_eq!(after_sample(SAMPLE_ATTEMPTS, false), Sampling::GiveUp);
        assert_eq!(after_sample(SAMPLE_ATTEMPTS, true), Sampling::Observed);
    }
}
