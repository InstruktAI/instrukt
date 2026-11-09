//! CI observations and failed-check evidence from GitHub (`integration-gate`, "Candidate and
//! verification"; `event-activation`, "Sources and reactions").
//!
//! A commit's result is read from its combined status and its check runs:
//! green when the combined status is `success` (or there are no statuses)
//! and every check run completed as `success`, `neutral` or `skipped`, with
//! at least one status or check run; red when any status or check run
//! failed; pending otherwise, including when nothing has reported yet.
//!
//! Requests are conditional: the `ETag` of each answer is kept with its
//! body in the runner's state, so an unchanged `304` costs no rate quota.
//! A source that sent `X-Poll-Interval` is not asked again sooner.

use std::collections::BTreeMap;
use std::time::Duration;

use instrukt_core::ids::{CommitSha, ProjectSlug};
use jiff::{SignedDuration, Timestamp};
use serde::Deserialize;

use crate::events::{CiResult, Event};
use crate::state::{self, RunnerState, StateError};
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};

/// How long one GitHub request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A commit's CI state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiState {
    Green,
    Red,
    /// Something is still running.
    Pending,
    /// No status and no check run exists for the commit.
    NothingReported,
}

impl CiState {
    /// The completed result an event carries; `None` while pending.
    pub fn completed(self) -> Option<CiResult> {
        match self {
            CiState::Green => Some(CiResult::Success),
            CiState::Red => Some(CiResult::Failure),
            CiState::Pending | CiState::NothingReported => None,
        }
    }
}

/// The candidate's result: its own CI's completed result, or — only while
/// nothing has reported for the candidate — "no CI" when the canonical
/// head carries no status and no check run either. A candidate with nothing
/// reported on a project whose canonical head has CI is pending.
pub fn candidate_result(candidate: CiState, canonical_head: Option<CiState>) -> Option<CiResult> {
    match (candidate, canonical_head) {
        (CiState::NothingReported, Some(CiState::NothingReported)) => Some(CiResult::NoCi),
        (state, _) => state.completed(),
    }
}

/// The events one CI read yields: `runner.ci_completed` for the active
/// candidate's completed result (or `no_ci`, decided from canonical main's
/// head), and `runner.main_failing` when canonical main's head itself is
/// red. Only the current head is ever read, so a red result on a commit
/// that is no longer the head is not observed; the event log's idempotency
/// reports a head's red result once and a new red head afresh.
pub fn events(
    project: &ProjectSlug,
    candidate: Option<&CommitSha>,
    candidate_state: Option<CiState>,
    head: Option<&CommitSha>,
    head_state: Option<CiState>,
) -> Vec<Event> {
    let mut events = Vec::new();
    if let (Some(candidate), Some(state)) = (candidate, candidate_state)
        && let Some(result) = candidate_result(state, head_state)
    {
        events.push(Event::CiCompleted {
            project: project.clone(),
            candidate: candidate.clone(),
            result,
        });
    }
    if let (Some(head), Some(CiState::Red)) = (head, head_state) {
        events.push(Event::MainFailing {
            project: project.clone(),
            head: head.clone(),
        });
    }
    events
}

/// GitHub's combined status of a commit, as far as the result needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CombinedStatus {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub statuses: Vec<StatusEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StatusEntry {
    pub state: String,
}

/// GitHub's check runs of a commit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CheckRuns {
    pub total_count: usize,
    #[serde(default)]
    pub check_runs: Vec<CheckRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CheckRun {
    pub id: u64,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
}

/// Classifies a commit's statuses and check runs.
pub fn classify(status: &CombinedStatus, checks: &CheckRuns) -> CiState {
    let status_failed = status
        .statuses
        .iter()
        .any(|s| matches!(s.state.as_str(), "failure" | "error"))
        || (!status.statuses.is_empty() && matches!(status.state.as_str(), "failure" | "error"));
    let check_failed = checks.check_runs.iter().any(CheckRun::failed);
    if status_failed || check_failed {
        return CiState::Red;
    }
    if status.statuses.is_empty() && checks.check_runs.is_empty() {
        return CiState::NothingReported;
    }
    let statuses_green = status.statuses.is_empty() || status.state == "success";
    let checks_green = checks
        .check_runs
        .iter()
        .all(|run| run.status == "completed");
    if statuses_green && checks_green {
        CiState::Green
    } else {
        CiState::Pending
    }
}

/// A CI read could not be made.
#[derive(Debug, thiserror::Error)]
pub enum CiError {
    #[error("GitHub request {url} failed: {message}")]
    Request { url: String, message: String },
    #[error("GitHub answered {url} with {status}")]
    Status { url: String, status: u16 },
    #[error("GitHub's answer to {url} is not readable: {message}")]
    Body { url: String, message: String },
    #[error(transparent)]
    State(#[from] StateError),
}

/// What one candidate read found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiRead {
    /// The source asked not to be polled before this time.
    Deferred(Timestamp),
    /// The state of each commit asked for, in order.
    States(Vec<CommitChecks>),
}

/// One observed commit with the original failed check identities for delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitChecks {
    pub state: CiState,
    pub failed_checks: Vec<CheckRun>,
}

impl CheckRun {
    fn failed(&self) -> bool {
        self.status == "completed"
            && !matches!(
                self.conclusion.as_deref(),
                Some("success" | "neutral" | "skipped")
            )
    }
}

/// `project`'s conditional-request validators and poll deferral.
fn load_validators(state: &RunnerState, project: &ProjectSlug) -> Result<Validators, StateError> {
    let loaded = state.read(|conn, path| {
        let sql = state::sql(path);
        let mut statement = conn
            .prepare("SELECT request, etag, body FROM request_validators WHERE project = ?1")
            .map_err(&sql)?;
        let answers = statement
            .query_map(params![project.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    Cached {
                        etag: row.get(1)?,
                        body: row.get(2)?,
                    },
                ))
            })
            .map_err(&sql)?
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(&sql)?;
        let not_before: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT until, cause FROM poll_deferrals WHERE project = ?1",
                params![project.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(&sql)?;
        Ok(Validators {
            answers,
            cause: not_before.as_ref().and_then(|(_, cause)| cause.clone()),
            not_before: not_before
                .map(|(text, _)| {
                    text.parse::<Timestamp>()
                        .map_err(|e| state::invalid(path, e))
                })
                .transpose()?,
        })
    })?;
    Ok(loaded.unwrap_or_default())
}

/// Replaces `project`'s validators and poll deferral in one transaction.
fn save_validators(
    state: &RunnerState,
    project: &ProjectSlug,
    validators: &Validators,
) -> Result<(), StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        let sql = state::sql(&path);
        for (request, cached) in &validators.answers {
            tx.execute(
                "INSERT INTO request_validators (project, request, etag, body) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (project, request) DO UPDATE SET etag = excluded.etag, body = excluded.body
                 WHERE etag IS NOT excluded.etag OR body IS NOT excluded.body",
                params![project.as_str(), request, cached.etag, cached.body],
            )
            .map_err(&sql)?;
        }
        match validators.not_before {
            Some(until) => tx.execute(
                "INSERT INTO poll_deferrals (project, until, cause) VALUES (?1, ?2, ?3)
                 ON CONFLICT (project) DO UPDATE SET until = excluded.until, cause = excluded.cause",
                params![project.as_str(), until.to_string(), validators.cause],
            ),
            None => tx.execute(
                "DELETE FROM poll_deferrals WHERE project = ?1",
                params![project.as_str()],
            ),
        }
        .map_err(&sql)?;
        Ok(())
    })
}

#[derive(Debug, Clone, Default)]
struct Validators {
    answers: BTreeMap<String, Cached>,
    not_before: Option<Timestamp>,
    cause: Option<String>,
}

#[derive(Debug, Clone)]
struct Cached {
    etag: String,
    body: String,
}

/// GitHub's commit status API for one project's repository.
pub struct CiClient<'a> {
    api_url: String,
    repository: String,
    token: String,
    state: &'a RunnerState,
    project: &'a ProjectSlug,
}

impl<'a> CiClient<'a> {
    pub fn new(
        api_url: &str,
        repository: &str,
        token: String,
        state: &'a RunnerState,
        project: &'a ProjectSlug,
    ) -> Self {
        Self {
            api_url: api_url.trim_end_matches('/').to_owned(),
            repository: repository.to_owned(),
            token,
            state,
            project,
        }
    }

    /// Reads each of `commits`' CI states at `now`, unless the source
    /// deferred. One poll interval covers the whole read.
    pub fn read(&self, commits: &[&CommitSha], now: Timestamp) -> Result<CiRead, CiError> {
        let mut validators = load_validators(self.state, self.project)?;
        if let Some(not_before) = validators.not_before
            && now < not_before
        {
            return Ok(CiRead::Deferred(not_before));
        }
        let mut interval = None;
        validators.cause = None;
        let mut bodies = Vec::with_capacity(commits.len());
        let mut failure = None;
        for commit in commits {
            let base = format!(
                "{}/repos/{}/commits/{commit}",
                self.api_url, self.repository
            );
            let read = self
                .get(&format!("{base}/status"), &mut validators, &mut interval)
                .and_then(|status| {
                    self.check_runs(
                        &format!("{base}/check-runs"),
                        &mut validators,
                        &mut interval,
                    )
                    .map(|checks| (base, status, checks))
                });
            match read {
                Ok(read) => bodies.push(read),
                Err(err) => {
                    failure = Some(err);
                    break;
                }
            }
        }
        // A source's poll interval binds whether or not its answer was
        // usable: an error carrying `X-Poll-Interval` still defers the
        // next request.
        validators.not_before =
            interval.and_then(|secs: i64| now.checked_add(SignedDuration::from_secs(secs)).ok());
        save_validators(self.state, self.project, &validators)?;
        if let Some(err) = failure {
            return Err(err);
        }
        let mut states = Vec::with_capacity(bodies.len());
        for (base, status_body, checks_body) in bodies {
            let status: CombinedStatus = parse(&format!("{base}/status"), &status_body)?;
            let checks = checks_body;
            states.push(CommitChecks {
                state: classify(&status, &checks),
                failed_checks: checks
                    .check_runs
                    .into_iter()
                    .filter(CheckRun::failed)
                    .collect(),
            });
        }
        Ok(CiRead::States(states))
    }

    fn check_runs(
        &self,
        url: &str,
        validators: &mut Validators,
        interval: &mut Option<i64>,
    ) -> Result<CheckRuns, CiError> {
        let mut checks: CheckRuns = parse(url, &self.get(url, validators, interval)?)?;
        let mut page = 2;
        while checks.check_runs.len() < checks.total_count {
            let next_url = format!("{url}?page={page}");
            let next: CheckRuns = parse(&next_url, &self.get(&next_url, validators, interval)?)?;
            if next.check_runs.is_empty() {
                return Err(CiError::Body {
                    url: next_url,
                    message: "check-run pagination ended before its reported total".to_owned(),
                });
            }
            checks.total_count = checks.total_count.max(next.total_count);
            checks.check_runs.extend(next.check_runs);
            page += 1;
        }
        Ok(checks)
    }

    fn get(
        &self,
        url: &str,
        validators: &mut Validators,
        interval: &mut Option<i64>,
    ) -> Result<String, CiError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .redirect_auth_headers(ureq::config::RedirectAuthHeaders::SameHost)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .build()
            .into();
        let mut request = agent
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "instrukt");
        request = request.header("Authorization", &format!("Bearer {}", self.token));
        if let Some(cached) = validators.answers.get(url) {
            request = request.header("If-None-Match", &cached.etag);
        }
        let mut response = request.call().map_err(|err| CiError::Request {
            url: url.to_owned(),
            message: err.to_string(),
        })?;
        if let Some(secs) = response
            .headers()
            .get("X-Poll-Interval")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<i64>().ok())
        {
            *interval = Some(interval.map_or(secs, |known| known.max(secs)));
        }
        let status = response.status().as_u16();
        if status == 304
            && let Some(cached) = validators.answers.get(url)
        {
            tracing::debug!(event = "ci_not_modified", url);
            return Ok(cached.body.clone());
        }
        if !(200..300).contains(&status) {
            return Err(CiError::Status {
                url: url.to_owned(),
                status,
            });
        }
        let etag = response
            .headers()
            .get("ETag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|err| CiError::Body {
                url: url.to_owned(),
                message: err.to_string(),
            })?;
        match etag {
            Some(etag) => {
                validators.answers.insert(
                    url.to_owned(),
                    Cached {
                        etag,
                        body: body.clone(),
                    },
                );
            }
            None => {
                validators.answers.remove(url);
            }
        }
        Ok(body)
    }
}

fn parse<T: serde::de::DeserializeOwned>(url: &str, body: &str) -> Result<T, CiError> {
    serde_json::from_str(body).map_err(|err| CiError::Body {
        url: url.to_owned(),
        message: err.to_string(),
    })
}

/// Honor the observation source's deadline before even attempting credentials.
pub(crate) fn credential_deferral(
    state: &RunnerState,
    project: &ProjectSlug,
    now: Timestamp,
) -> Result<Option<Timestamp>, StateError> {
    Ok(load_validators(state, project)?
        .not_before
        .filter(|until| *until > now))
}
pub(crate) fn record_credential_failure(
    state: &RunnerState,
    project: &ProjectSlug,
    error: &instrukt_core::github::AppError,
    now: Timestamp,
) -> Result<(), StateError> {
    let mut validators = load_validators(state, project)?;
    validators.not_before = Some(match error {
        instrukt_core::github::AppError::RateLimited { until: Some(until) } => *until,
        _ => now,
    });
    validators.cause = Some(error.to_string());
    save_validators(state, project, &validators)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: &str, entries: &[&str]) -> CombinedStatus {
        CombinedStatus {
            state: state.into(),
            statuses: entries
                .iter()
                .map(|s| StatusEntry { state: (*s).into() })
                .collect(),
        }
    }

    fn checks(runs: &[(&str, Option<&str>)]) -> CheckRuns {
        CheckRuns {
            total_count: runs.len(),
            check_runs: runs
                .iter()
                .map(|(status, conclusion)| CheckRun {
                    id: 1,
                    name: "test".into(),
                    status: (*status).into(),
                    conclusion: conclusion.map(str::to_owned),
                })
                .collect(),
        }
    }

    #[test]
    fn nothing_reported_is_pending_not_green() {
        assert_eq!(
            classify(&status("pending", &[]), &checks(&[])),
            CiState::NothingReported
        );
    }

    #[test]
    fn green_needs_a_successful_status_or_none_and_every_check_concluded_well() {
        assert_eq!(
            classify(&status("success", &["success"]), &checks(&[])),
            CiState::Green
        );
        assert_eq!(
            classify(
                &status("pending", &[]),
                &checks(&[
                    ("completed", Some("success")),
                    ("completed", Some("skipped")),
                    ("completed", Some("neutral"))
                ])
            ),
            CiState::Green
        );
        assert_eq!(
            classify(
                &status("success", &["success"]),
                &checks(&[("in_progress", None)])
            ),
            CiState::Pending
        );
        assert_eq!(
            classify(&status("pending", &["pending"]), &checks(&[])),
            CiState::Pending
        );
    }

    #[test]
    fn any_failure_is_red() {
        assert_eq!(
            classify(&status("failure", &["failure"]), &checks(&[])),
            CiState::Red
        );
        assert_eq!(
            classify(&status("error", &["error", "pending"]), &checks(&[])),
            CiState::Red
        );
        assert_eq!(
            classify(
                &status("success", &["success"]),
                &checks(&[("completed", Some("timed_out")), ("queued", None)])
            ),
            CiState::Red
        );
        assert_eq!(CiState::Red.completed(), Some(CiResult::Failure));
        assert_eq!(CiState::Pending.completed(), None);
    }

    #[test]
    fn no_ci_is_decided_from_the_canonical_head_never_from_the_candidate() {
        use CiState::{Green, NothingReported, Pending, Red};
        assert_eq!(
            candidate_result(NothingReported, Some(NothingReported)),
            Some(CiResult::NoCi)
        );
        assert_eq!(candidate_result(NothingReported, Some(Green)), None);
        assert_eq!(candidate_result(NothingReported, Some(Pending)), None);
        assert_eq!(candidate_result(NothingReported, None), None);
        assert_eq!(candidate_result(Pending, Some(NothingReported)), None);
        assert_eq!(
            candidate_result(Green, Some(NothingReported)),
            Some(CiResult::Success)
        );
        assert_eq!(candidate_result(Red, Some(Green)), Some(CiResult::Failure));
    }

    fn sha(c: char) -> CommitSha {
        CommitSha::parse(&c.to_string().repeat(40)).unwrap()
    }

    #[test]
    fn a_red_canonical_head_is_reported_as_failing_and_nothing_else_is() {
        let project = ProjectSlug::parse("demo").unwrap();
        let head = sha('a');
        assert_eq!(
            events(&project, None, None, Some(&head), Some(CiState::Red)),
            [Event::MainFailing {
                project: project.clone(),
                head: head.clone()
            }]
        );
        for state in [CiState::Green, CiState::Pending, CiState::NothingReported] {
            assert!(events(&project, None, None, Some(&head), Some(state)).is_empty());
        }
        let candidate = sha('b');
        assert_eq!(
            events(
                &project,
                Some(&candidate),
                Some(CiState::Green),
                Some(&head),
                Some(CiState::Red)
            ),
            [
                Event::CiCompleted {
                    project: project.clone(),
                    candidate: candidate.clone(),
                    result: CiResult::Success
                },
                Event::MainFailing {
                    project: project.clone(),
                    head
                },
            ]
        );
    }
}
