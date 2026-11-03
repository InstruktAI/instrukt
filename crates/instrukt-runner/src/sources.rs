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

/// Reads local main, remote main and the active candidate with local git.
pub fn observe_main(root: &Path) -> Result<MainObservation, SourceError> {
    let branch = git::default_branch(root, CANONICAL_REMOTE)?;
    let local_main = resolve(root, &format!("refs/heads/{branch}"))?;
    let remote_main = resolve(root, &format!("refs/remotes/{CANONICAL_REMOTE}/{branch}"))?;
    let integrate = resolve(
        root,
        &format!("refs/remotes/{CANONICAL_REMOTE}/{INTEGRATE_BRANCH}"),
    )?;
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
}
