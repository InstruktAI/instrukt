//! Automatic review's requests as the runner holds them
//! (`project/instrukt/spec/feature/runner/pull-request-review`, "One review
//! per commit"): what work reads when it admits a verdict. Each answer comes
//! from delivery's own records, the selected runtime or the history store,
//! read without a lock.

mod _conversation;
mod _eligibility;
mod _observation;
mod _requests;

use std::collections::{BTreeMap, BTreeSet};

use instrukt_core::home::Home;
use instrukt_core::ids::CommitSha;
use jiff::Timestamp;

use crate::event_log::DbEventLog;
use crate::events::Event;
use crate::interfaces::{AutomaticReview, ProjectRef};
use crate::state::RunnerState;

pub use _conversation::{Conversation, ConversationHolder, Message, conversation};
pub use _requests::{
    Disposition, Request, Standing, TurnOpening, requests, turn_opening, turn_opening_with,
};

/// A review answer could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ReviewReadError {
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error("a review request's record is unreadable: {0}")]
    Record(String),
}

/// One opted-in project's review observation in a pass, with the
/// observation credential readiness issued.
pub(crate) struct ReviewPass<'a> {
    pub home: &'a Home,
    pub state: &'a RunnerState,
    pub log: &'a DbEventLog,
    pub project: &'a ProjectRef,
    pub api_url: &'a str,
    pub repository: &'a str,
    pub token: &'a str,
    pub port: &'a dyn AutomaticReview,
    pub now: Timestamp,
}

/// The remote the project's checkout is fetched from.
const REMOTE: &str = "origin";

/// Observes the project's open pull requests and asks for the reviews that
/// are owed (`pull-request-review`, Which heads are reviewed, Review
/// requests, One review per commit).
pub(crate) fn observe(pass: &ReviewPass<'_>) -> Result<(), String> {
    let project = &pass.project.slug;
    let mut interval = None;
    let listed = _observation::open_pulls(
        pass.home,
        project,
        pass.api_url,
        pass.repository,
        pass.token,
        &mut interval,
    );
    if let Some(seconds) = interval
        && let Err(error) = crate::ci::defer_polling(pass.state, project, seconds, pass.now)
    {
        tracing::warn!(event = "ci_observation_state_failed", error = %error);
    }
    let pulls = listed.map_err(|error| error.to_string())?;
    let identities = pass.port.identities()?;
    let todo_pulls = pass.port.todo_pulls(project)?;
    let mut heads: BTreeMap<CommitSha, Vec<_eligibility::Eligible>> = BTreeMap::new();
    for pull in &pulls {
        match _eligibility::eligibility(pull, pass.repository, &identities, &todo_pulls) {
            Ok(eligible) => heads
                .entry(eligible.head.clone())
                .or_default()
                .push(eligible),
            Err(reason) => {
                tracing::debug!(event = "review_pull_ineligible", project = %project, pull = pull["number"].as_u64(), reason = ?reason);
            }
        }
    }
    let eligible: Vec<(u64, CommitSha)> = heads
        .iter()
        .flat_map(|(head, at)| at.iter().map(move |pull| (pull.number, head.clone())))
        .collect();
    if let Err(error) = pass
        .port
        .publish(project, pass.repository, &eligible, pass.now)
    {
        tracing::warn!(event = "review_publication_failed", project = %project, error = %error);
    }
    let mut owed = BTreeSet::new();
    let mut fetched = false;
    for (head, mut at_head) in heads {
        if pass.port.carries_verdict(project, &head)? {
            continue;
        }
        owed.insert(head.clone());
        let requests = requests(pass.state, project, &head).map_err(|e| e.to_string())?;
        if requests.iter().any(_requests::live) || !_requests::due(&requests, pass.now) {
            continue;
        }
        if !fetched {
            // The request's commits must be present where its consumer runs.
            instrukt_core::git::run(&pass.project.root, ["fetch", "--quiet", REMOTE])
                .map_err(|error| error.to_string())?;
            fetched = true;
        }
        at_head.sort_by_key(|eligible| eligible.number);
        let addressed = &at_head[0];
        let Some(base) = pinned_base(&pass.project.root, &head, &addressed.base_branch)? else {
            tracing::info!(event = "review_request_unpinned", project = %project, head = %head, "no merge base with the canonical branch or the pull request's base branch");
            continue;
        };
        let request = Event::ReviewNeeded {
            project: project.clone(),
            pull_request: Some(addressed.number),
            head_branch: Some(addressed.head_branch.clone()),
            base_branch: Some(addressed.base_branch.clone()),
            head: head.clone(),
            base,
            request: requests.len() as u64 + 1,
        };
        pass.log.log(&request).map_err(|error| error.to_string())?;
        tracing::info!(event = "review_request_emitted", project = %project, head = %head, pull = addressed.number, request = requests.len() + 1);
    }
    _requests::end_unowed(pass.state, project, &owed).map_err(|error| error.to_string())
}

/// The pinned base of `head` in the repository at `root`: its merge base
/// with the canonical branch, else with `base_branch`; none when it shares
/// history with neither.
fn pinned_base(
    root: &std::path::Path,
    head: &CommitSha,
    base_branch: &str,
) -> Result<Option<CommitSha>, String> {
    let canonical = instrukt_core::git::default_branch(root, REMOTE).map_err(|e| e.to_string())?;
    for branch in [canonical.as_str(), base_branch] {
        // A branch name from GitHub reaches git as one argument behind a
        // fixed prefix, never as an option or through a shell.
        let tracked = format!("refs/remotes/{REMOTE}/{branch}");
        let output =
            instrukt_core::git::run_output(root, ["merge-base", head.as_str(), tracked.as_str()])
                .map_err(|error| error.to_string())?;
        if output.status.success()
            && let Ok(base) = CommitSha::parse(String::from_utf8_lossy(&output.stdout).trim())
        {
            return Ok(Some(base));
        }
    }
    Ok(None)
}
