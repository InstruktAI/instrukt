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
    Disposition, Request, Standing, TurnOpening, observed_red, requests, turn_opening,
    turn_opening_with,
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
    /// The active integration candidate, when there is one.
    pub candidate: Option<&'a CommitSha>,
    pub state: &'a RunnerState,
    pub log: &'a DbEventLog,
    pub project: &'a ProjectRef,
    pub api_url: &'a str,
    pub repository: &'a str,
    pub token: &'a str,
    pub port: &'a dyn AutomaticReview,
    /// The moment the wake began.
    pub now: Timestamp,
    /// The operator's clock, read again where publication's credential is
    /// issued: the pass's own requests come first.
    pub clock: &'a dyn Fn() -> Timestamp,
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
    if let Err(error) = pass.port.publish(
        project,
        pass.repository,
        &eligible,
        pass.candidate,
        pass.now,
        pass.clock,
    ) {
        tracing::warn!(event = "review_publication_failed", project = %project, error = %error);
    }
    // The candidate's coverage is derived before any pull request's head is
    // asked for: a covered candidate carries its derived approval, so a pull
    // request at the same head starts no review of it (`github-apps`,
    // Verdict publication).
    let finding = match pass.candidate {
        Some(candidate) => derive_candidate(pass, candidate, &eligible)?,
        None => None,
    };
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
        at_head.sort_by_key(|eligible| eligible.number);
        emit(pass, &head, Some(&at_head[0]), requests.len(), &mut fetched)?;
    }
    _requests::end_unowed(pass.state, project, &owed).map_err(|error| error.to_string())?;
    if let (Some(candidate), Some(crate::interfaces::CandidateFinding::Owed)) =
        (pass.candidate, &finding)
    {
        ask_candidate(pass, candidate, &mut fetched)?;
    }
    if let Some(owed) = owed_candidates(pass.candidate, finding.as_ref()) {
        _requests::end_unowed_candidates(pass.state, project, &owed).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The candidate commits still owed a review after this pass, whose queued
/// requests stay; every other queued candidate request ends
/// (`pull-request-review`, One review per commit: a queued request ends when
/// its commit stops being owed). Without an active candidate, or with one
/// whose request is selected or handed over, none is. Nothing is decided
/// while the repository cannot answer.
fn owed_candidates(
    candidate: Option<&CommitSha>,
    finding: Option<&crate::interfaces::CandidateFinding>,
) -> Option<BTreeSet<CommitSha>> {
    use crate::interfaces::CandidateFinding;
    match (candidate, finding) {
        (_, Some(CandidateFinding::CannotAnswer(_))) => None,
        (Some(candidate), Some(CandidateFinding::Owed)) => {
            Some(BTreeSet::from([candidate.clone()]))
        }
        _ => Some(BTreeSet::new()),
    }
}

/// Work's finding on the active candidate, derived while no request of it
/// is selected or handed over (`integration-gate`, Review coverage of a
/// candidate); none while one is.
fn derive_candidate(
    pass: &ReviewPass<'_>,
    candidate: &CommitSha,
    eligible: &[(u64, CommitSha)],
) -> Result<Option<crate::interfaces::CandidateFinding>, String> {
    let project = &pass.project.slug;
    let requests = requests(pass.state, project, candidate).map_err(|e| e.to_string())?;
    if requests
        .iter()
        .any(|r| matches!(r.standing, Standing::Selected | Standing::HandedOver))
    {
        return Ok(None);
    }
    let pulls: Vec<u64> = eligible
        .iter()
        .filter(|(_, head)| head == candidate)
        .map(|(number, _)| *number)
        .collect();
    let finding = pass.port.candidate(
        pass.project,
        pass.repository,
        candidate,
        &pulls,
        pass.now,
        pass.clock,
    )?;
    tracing::info!(event = "review_candidate_derived", project = %project, candidate = %candidate, finding = ?finding);
    Ok(Some(finding))
}

/// Asks for an uncovered candidate (`pull-request-review`, Candidate
/// requests), unless a request of it is live or the next is not due yet.
fn ask_candidate(
    pass: &ReviewPass<'_>,
    candidate: &CommitSha,
    fetched: &mut bool,
) -> Result<(), String> {
    let requests =
        requests(pass.state, &pass.project.slug, candidate).map_err(|e| e.to_string())?;
    if !requests.iter().any(_requests::live) && _requests::due(&requests, pass.now) {
        emit(pass, candidate, None, requests.len(), fetched)?;
    }
    Ok(())
}

/// Emits the next review request of `head`: addressed to `pull` for a pull
/// request's head, to the integration branch for a candidate. The project's
/// checkout is fetched once per pass first.
fn emit(
    pass: &ReviewPass<'_>,
    head: &CommitSha,
    pull: Option<&_eligibility::Eligible>,
    previous: usize,
    fetched: &mut bool,
) -> Result<(), String> {
    let project = &pass.project.slug;
    if !*fetched {
        // The request's commits must be present where its consumer runs.
        instrukt_core::git::run(&pass.project.root, ["fetch", "--quiet", REMOTE])
            .map_err(|error| error.to_string())?;
        *fetched = true;
    }
    let base_branch = pull.map(|pull| pull.base_branch.as_str());
    let Some(base) = pinned_base(&pass.project.root, head, base_branch)? else {
        tracing::info!(event = "review_request_unpinned", project = %project, head = %head, "no merge base with the canonical branch or the pull request's base branch");
        return Ok(());
    };
    let number = previous as u64 + 1;
    let request = Event::ReviewNeeded {
        project: project.clone(),
        pull_request: pull.map(|pull| pull.number),
        head_branch: pull.map(|pull| pull.head_branch.clone()),
        base_branch: base_branch.map(str::to_owned),
        head: head.clone(),
        base,
        request: number,
    };
    pass.log.log(&request).map_err(|error| error.to_string())?;
    tracing::info!(event = "review_request_emitted", project = %project, head = %head, pull = pull.map(|p| p.number), request = number);
    Ok(())
}

/// The pinned base of `head` in the repository at `root`: its merge base
/// with the canonical branch, else with `base_branch`; none when it shares
/// history with neither.
fn pinned_base(
    root: &std::path::Path,
    head: &CommitSha,
    base_branch: Option<&str>,
) -> Result<Option<CommitSha>, String> {
    let canonical = instrukt_core::git::default_branch(root, REMOTE).map_err(|e| e.to_string())?;
    for branch in std::iter::once(canonical.as_str()).chain(base_branch) {
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

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::sync::Mutex;

    use instrukt_core::ids::ProjectSlug;

    use super::*;
    use crate::interfaces::{CandidateFinding, ProjectRef, ReviewIdentities};

    fn sha(c: char) -> CommitSha {
        CommitSha::parse(&c.to_string().repeat(40)).unwrap()
    }

    /// A GitHub that answers one pull-request listing with no pull request.
    fn listing_no_pulls() -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let served = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while !request.windows(4).any(|end| end == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0, "the listing request ended early");
                request.extend_from_slice(&chunk[..read]);
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                      Content-Length: 2\r\nConnection: close\r\n\r\n[]",
                )
                .unwrap();
        });
        (url, served)
    }

    /// Work holding no verdict, noting the moment publication and candidate
    /// derivation would issue their credential.
    #[derive(Default)]
    struct Issuing {
        issued: Mutex<Vec<(&'static str, Timestamp)>>,
    }

    impl AutomaticReview for Issuing {
        fn readiness(
            &self,
            _repository: &str,
            _now: Timestamp,
        ) -> instrukt_core::interfaces::ReviewReadiness {
            // Asked before the review pass, never within it.
            instrukt_core::interfaces::ReviewReadiness::CannotAsk("not asked here".into())
        }
        fn identities(&self) -> Result<ReviewIdentities, String> {
            Ok(ReviewIdentities {
                trusted_bots: Vec::new(),
                reviewer: instrukt_core::github::Bot {
                    id: 1,
                    node_id: "BOT_1".into(),
                    login: "reviewer[bot]".into(),
                },
            })
        }
        fn todo_pulls(&self, _project: &ProjectSlug) -> Result<BTreeSet<u64>, String> {
            Ok(BTreeSet::new())
        }
        fn carries_verdict(
            &self,
            _project: &ProjectSlug,
            _head: &CommitSha,
        ) -> Result<bool, String> {
            Ok(false)
        }
        fn needs_work(&self, _project: &ProjectSlug, _commit: &CommitSha) -> Result<bool, String> {
            Ok(false)
        }
        fn candidate(
            &self,
            _project: &ProjectRef,
            _repository: &str,
            _candidate: &CommitSha,
            _pulls: &[u64],
            _now: Timestamp,
            clock: &dyn Fn() -> Timestamp,
        ) -> Result<CandidateFinding, String> {
            self.issued.lock().unwrap().push(("candidate", clock()));
            Ok(CandidateFinding::Settled)
        }
        fn publish(
            &self,
            _project: &ProjectSlug,
            _repository: &str,
            _eligible: &[(u64, CommitSha)],
            _candidate: Option<&CommitSha>,
            _now: Timestamp,
            clock: &dyn Fn() -> Timestamp,
        ) -> Result<(), String> {
            self.issued.lock().unwrap().push(("publish", clock()));
            Ok(())
        }
    }

    #[test]
    fn publication_and_candidate_derivation_issue_on_the_clock_not_when_the_wake_began() {
        let dir = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        let woke: Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
        // The pass's own requests have taken their time by then.
        let issuing: Timestamp = "2026-01-01T00:00:42Z".parse().unwrap();
        let state = RunnerState::new(&home);
        let log = DbEventLog::new(state.clone(), woke);
        let project = ProjectRef {
            slug: ProjectSlug::parse("demo").unwrap(),
            root: dir.path().join("repo"),
        };
        let (api_url, served) = listing_no_pulls();
        let port = Issuing::default();
        let candidate = sha('c');
        let clock = || issuing;
        observe(&ReviewPass {
            home: &home,
            candidate: Some(&candidate),
            state: &state,
            log: &log,
            project: &project,
            api_url: &api_url,
            repository: "acme/demo",
            token: "observation",
            port: &port,
            now: woke,
            clock: &clock,
        })
        .unwrap();
        served.join().unwrap();
        assert_eq!(
            *port.issued.lock().unwrap(),
            vec![("publish", issuing), ("candidate", issuing)]
        );
    }

    #[test]
    fn without_an_active_candidate_no_candidate_is_owed_a_review() {
        assert_eq!(owed_candidates(None, None), Some(BTreeSet::new()));
    }

    #[test]
    fn a_candidate_whose_request_is_in_flight_leaves_no_other_candidate_owed() {
        assert_eq!(
            owed_candidates(Some(&sha('a')), None),
            Some(BTreeSet::new())
        );
    }

    #[test]
    fn only_an_uncovered_candidate_is_owed_a_review() {
        let candidate = sha('a');
        assert_eq!(
            owed_candidates(Some(&candidate), Some(&CandidateFinding::Owed)),
            Some(BTreeSet::from([candidate.clone()]))
        );
        for covered in [CandidateFinding::Covered, CandidateFinding::Settled] {
            assert_eq!(
                owed_candidates(Some(&candidate), Some(&covered)),
                Some(BTreeSet::new())
            );
        }
    }

    #[test]
    fn nothing_is_decided_while_the_repository_cannot_answer() {
        assert_eq!(
            owed_candidates(
                Some(&sha('a')),
                Some(&CandidateFinding::CannotAnswer("busy".into()))
            ),
            None
        );
    }
}
