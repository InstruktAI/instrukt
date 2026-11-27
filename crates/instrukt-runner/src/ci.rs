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

/// What the candidate's review says of its result on a project that opted
/// in to automatic review.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReviewGate {
    /// The commit's current verdict with work is needs work.
    pub needs_work: bool,
    /// The state of the reviewer bot's review status the commit carries.
    pub status: Option<String>,
}

/// The candidate's result: on a project that opted in, its review decides
/// first (`integration-gate`, Candidate and verification): needs work
/// current with work fails it, no review status keeps it pending, a failure
/// status fails it. Then its own CI's completed result, or — only while
/// nothing has reported for the candidate — "no CI" when the canonical
/// head carries no status and no check run either. A candidate with nothing
/// reported on a project whose canonical head has CI is pending.
pub fn candidate_result(
    candidate: CiState,
    canonical_head: Option<CiState>,
    review: Option<&ReviewGate>,
) -> Option<CiResult> {
    if let Some(review) = review {
        if review.needs_work {
            return Some(CiResult::Failure);
        }
        match review.status.as_deref() {
            None => return None,
            Some("failure" | "error") => return Some(CiResult::Failure),
            Some(_) => {}
        }
    }
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
    candidate_review: Option<&ReviewGate>,
    head: Option<&CommitSha>,
    head_state: Option<CiState>,
) -> Vec<Event> {
    let mut events = Vec::new();
    if let (Some(candidate), Some(state)) = (candidate, candidate_state)
        && let Some(result) = candidate_result(state, head_state, candidate_review)
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
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub creator: Option<serde_json::Value>,
}

/// Reduces a commit's status listing, newest first, as GitHub lists every
/// status ever posted: the reviewer bot's statuses named `instrukt/review`
/// are set apart, the newest of them being the review status the commit
/// carries; of the rest the newest of each context is that context's
/// status, with the state GitHub's combined answer would derive.
pub fn reduce(
    listing: &[StatusEntry],
    reviewer: &instrukt_core::github::Bot,
) -> (CombinedStatus, Option<String>) {
    let mut review = None;
    let mut contexts = std::collections::BTreeSet::new();
    let mut statuses = Vec::new();
    for status in listing {
        let context = status.context.clone().unwrap_or_default();
        let by_reviewer = status
            .creator
            .as_ref()
            .is_some_and(|creator| reviewer.matches(creator));
        if context == REVIEW_STATUS_CONTEXT && by_reviewer {
            review.get_or_insert_with(|| status.state.clone());
            continue;
        }
        if contexts.insert(context) {
            statuses.push(status.clone());
        }
    }
    let state = if statuses
        .iter()
        .any(|s| matches!(s.state.as_str(), "failure" | "error"))
    {
        "failure"
    } else if statuses.iter().any(|s| s.state == "pending") {
        "pending"
    } else {
        "success"
    };
    (
        CombinedStatus {
            state: state.to_owned(),
            statuses,
        },
        review,
    )
}

/// The review status's context (`github-apps`, Review verdict on the
/// reviewed commit).
const REVIEW_STATUS_CONTEXT: &str = "instrukt/review";

/// A status listing's page size; a shorter page ends it.
const STATUS_PAGE: usize = 100;

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
    /// On a project that opted in, the state of the reviewer bot's review
    /// status the commit carries.
    pub review_status: Option<String>,
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

/// One kept answer: its validator and its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cached {
    pub(crate) etag: String,
    pub(crate) body: String,
}

/// GitHub's commit status API for one project's repository.
pub struct CiClient<'a> {
    api_url: String,
    repository: String,
    token: String,
    state: &'a RunnerState,
    project: &'a ProjectSlug,
    /// On a project that opted in, the reviewer identity's bot: statuses
    /// are read as a listing and its review status is set apart.
    reviewer: Option<instrukt_core::github::Bot>,
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
            reviewer: None,
        }
    }

    /// Reads statuses as a listing and sets the reviewer bot's review
    /// status apart, as on a project that opted in to automatic review.
    pub fn with_reviewer(mut self, reviewer: instrukt_core::github::Bot) -> Self {
        self.reviewer = Some(reviewer);
        self
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
            let status = match &self.reviewer {
                Some(_) => {
                    self.status_listing(&format!("{base}/statuses"), &mut validators, &mut interval)
                }
                None => self.get(&format!("{base}/status"), &mut validators, &mut interval),
            };
            let read = status.and_then(|status| {
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
            let (status, review_status) = match &self.reviewer {
                Some(reviewer) => {
                    let listing: Vec<StatusEntry> =
                        parse(&format!("{base}/statuses"), &status_body)?;
                    reduce(&listing, reviewer)
                }
                None => (parse(&format!("{base}/status"), &status_body)?, None),
            };
            let checks = checks_body;
            states.push(CommitChecks {
                review_status,
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

    /// Every status of a commit, newest first, read page by page under the
    /// pass's validators until a short page ends it; the pages joined as one
    /// JSON list.
    fn status_listing(
        &self,
        url: &str,
        validators: &mut Validators,
        interval: &mut Option<i64>,
    ) -> Result<String, CiError> {
        let mut all: Vec<serde_json::Value> = Vec::new();
        for page in 1.. {
            let paged = format!("{url}?per_page={STATUS_PAGE}&page={page}");
            let items: Vec<serde_json::Value> =
                parse(&paged, &self.get(&paged, validators, interval)?)?;
            let full = items.len() == STATUS_PAGE;
            all.extend(items);
            if !full {
                break;
            }
        }
        serde_json::to_string(&all).map_err(|err| CiError::Body {
            url: url.to_owned(),
            message: err.to_string(),
        })
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
        match conditional_get(url, &self.token, validators.answers.get(url), interval)? {
            Fetched::Unchanged { body } => {
                tracing::debug!(event = "ci_not_modified", url);
                Ok(body)
            }
            Fetched::Fresh { etag, body } => {
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
    }
}

/// What one conditional GitHub request answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fetched {
    /// `304`: the validator sent still matches; this is the kept body.
    Unchanged { body: String },
    /// A fresh `2xx` body with the validator GitHub sent for it, if any.
    Fresh { etag: Option<String>, body: String },
}

/// One conditional `GET` of a GitHub API `url` under `token`, executed through
/// [`octocrab::Octocrab`] and [`instrukt_outside::runtime::block_on`]. `cached`
/// is the answer the caller keeps for `url`; a `304` is `Unchanged` only when
/// one was sent. `X-Poll-Interval` and `Retry-After` (parsed via
/// [`instrukt_outside::http::retry_after`]) raise `interval` whether or not the
/// answer is usable.
pub(crate) fn conditional_get(
    url: &str,
    token: &str,
    cached: Option<&Cached>,
    interval: &mut Option<i64>,
) -> Result<Fetched, CiError> {
    let req_err = |msg: String| CiError::Request {
        url: url.to_owned(),
        message: msg,
    };
    let uri: http::Uri = url
        .parse()
        .map_err(|e: http::uri::InvalidUri| req_err(e.to_string()))?;
    let (Some(scheme), Some(authority)) = (uri.scheme_str(), uri.authority()) else {
        return Err(req_err("GitHub URL is missing scheme or host".to_owned()));
    };
    let origin = format!("{scheme}://{authority}");
    let route = uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str)
        .to_owned();
    let crab = {
        let _guard = instrukt_outside::runtime::runtime()
            .map_err(|e| req_err(e.to_string()))?
            .enter();
        let mut builder = octocrab::Octocrab::builder()
            .base_uri(&origin)
            .map_err(|e| req_err(e.to_string()))?
            .set_connect_timeout(Some(Duration::from_secs(10)))
            .set_read_timeout(Some(REQUEST_TIMEOUT))
            .set_write_timeout(Some(REQUEST_TIMEOUT))
            .add_header(http::header::USER_AGENT, "instrukt".to_owned())
            .add_header(
                http::header::ACCEPT,
                "application/vnd.github+json".to_owned(),
            );
        if !token.is_empty() {
            if http::HeaderValue::from_str(&format!("Bearer {token}")).is_err() {
                return Err(CiError::Status {
                    url: url.to_owned(),
                    status: 401,
                });
            }
            builder = builder.personal_token(token.to_owned());
        }
        builder.build().map_err(|e| req_err(e.to_string()))?
    };
    let headers = cached.and_then(|entry| {
        let mut map = http::HeaderMap::new();
        if let Ok(tag) = entry.etag.parse::<octocrab::etag::EntityTag>()
            && octocrab::etag::EntityTag::insert_if_none_match_header(&mut map, tag).is_ok()
        {
            return Some(map);
        }
        map.insert(
            http::header::IF_NONE_MATCH,
            http::HeaderValue::from_str(&entry.etag).ok()?,
        );
        Some(map)
    });
    let (url_owned, timeout_url) = (url.to_owned(), url.to_owned());
    let (status, resp_headers, etag, body) = instrukt_outside::runtime::block_on(async move {
        tokio::time::timeout(REQUEST_TIMEOUT, async move {
            let resp =
                crab._get_with_headers(route, headers)
                    .await
                    .map_err(|e| CiError::Request {
                        url: url_owned.clone(),
                        message: e.to_string(),
                    })?;
            let status = resp.status().as_u16();
            let resp_headers = resp.headers().clone();
            let etag = octocrab::etag::EntityTag::extract_from_response(&resp)
                .map(|t| t.to_string())
                .or_else(|| {
                    resp_headers
                        .get(http::header::ETAG)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned)
                });
            let body = if status == 304 || !(200..300).contains(&status) {
                String::new()
            } else {
                crab.body_to_string(resp).await.map_err(|e| CiError::Body {
                    url: url_owned.clone(),
                    message: e.to_string(),
                })?
            };
            Ok::<_, CiError>((status, resp_headers, etag, body))
        })
        .await
        .map_err(|_| CiError::Request {
            url: timeout_url,
            message: "request timed out".to_owned(),
        })?
    })
    .map_err(|e| req_err(e.to_string()))??;

    let now = Timestamp::now();
    let poll_secs = resp_headers
        .get("X-Poll-Interval")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok());
    let retry_secs = instrukt_outside::http::retry_after(&resp_headers, now).map(|n| match n {
        instrukt_outside::Named::After { delay, .. } => {
            i64::try_from(delay.as_secs()).unwrap_or(i64::MAX)
        }
        instrukt_outside::Named::At(at) => (at.as_second() - now.as_second()).max(0),
    });
    for secs in poll_secs.into_iter().chain(retry_secs) {
        *interval = Some(interval.map_or(secs, |known| known.max(secs)));
    }
    if status == 304
        && let Some(cached) = cached
    {
        return Ok(Fetched::Unchanged {
            body: cached.body.clone(),
        });
    }
    if !(200..300).contains(&status) {
        return Err(CiError::Status {
            url: url.to_owned(),
            status,
        });
    }
    Ok(Fetched::Fresh { etag, body })
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
/// Records `cause` as the source's state without deferring it: the next
/// tick reads again.
pub(crate) fn record_source_cause(
    state: &RunnerState,
    project: &ProjectSlug,
    cause: &str,
    now: Timestamp,
) -> Result<(), StateError> {
    let mut validators = load_validators(state, project)?;
    validators.not_before = Some(validators.not_before.map_or(now, |until| until.max(now)));
    validators.cause = Some(cause.to_owned());
    save_validators(state, project, &validators)
}

/// Defers the source for `seconds` from `now`, unless it is already
/// deferred longer: a poll interval one of its reads was given.
pub(crate) fn defer_polling(
    state: &RunnerState,
    project: &ProjectSlug,
    seconds: i64,
    now: Timestamp,
) -> Result<(), StateError> {
    let Ok(until) = now.checked_add(SignedDuration::from_secs(seconds)) else {
        return Ok(());
    };
    let mut validators = load_validators(state, project)?;
    if validators.not_before.is_some_and(|known| known >= until) {
        return Ok(());
    }
    validators.not_before = Some(until);
    save_validators(state, project, &validators)
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
                .map(|s| StatusEntry {
                    state: (*s).into(),
                    context: None,
                    creator: None,
                })
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
            candidate_result(NothingReported, Some(NothingReported), None),
            Some(CiResult::NoCi)
        );
        assert_eq!(candidate_result(NothingReported, Some(Green), None), None);
        assert_eq!(candidate_result(NothingReported, Some(Pending), None), None);
        assert_eq!(candidate_result(NothingReported, None, None), None);
        assert_eq!(candidate_result(Pending, Some(NothingReported), None), None);
        assert_eq!(
            candidate_result(Green, Some(NothingReported), None),
            Some(CiResult::Success)
        );
        assert_eq!(
            candidate_result(Red, Some(Green), None),
            Some(CiResult::Failure)
        );
    }

    fn reviewer() -> instrukt_core::github::Bot {
        instrukt_core::github::Bot {
            id: 3,
            node_id: "BOT_3".into(),
            login: "reviewer[bot]".into(),
        }
    }

    fn listed(context: &str, state: &str, creator: (u64, &str)) -> StatusEntry {
        StatusEntry {
            state: state.into(),
            context: Some(context.into()),
            creator: Some(serde_json::json!({"id": creator.0, "node_id": creator.1})),
        }
    }

    fn gate(needs_work: bool, status: Option<&str>) -> ReviewGate {
        ReviewGate {
            needs_work,
            status: status.map(str::to_owned),
        }
    }

    #[test]
    fn a_needs_work_verdict_fails_the_candidate_whatever_status_it_carries() {
        assert_eq!(
            candidate_result(CiState::Green, None, Some(&gate(true, Some("success")))),
            Some(CiResult::Failure)
        );
    }

    #[test]
    fn a_candidate_without_a_review_status_is_pending() {
        assert_eq!(
            candidate_result(CiState::Green, None, Some(&gate(false, None))),
            None
        );
    }

    #[test]
    fn a_failing_review_status_fails_the_candidate() {
        assert_eq!(
            candidate_result(CiState::Green, None, Some(&gate(false, Some("failure")))),
            Some(CiResult::Failure)
        );
    }

    #[test]
    fn an_approving_review_status_leaves_the_ci_result() {
        assert_eq!(
            candidate_result(CiState::Red, None, Some(&gate(false, Some("success")))),
            Some(CiResult::Failure)
        );
        assert_eq!(
            candidate_result(CiState::Green, None, Some(&gate(false, Some("success")))),
            Some(CiResult::Success)
        );
    }

    #[test]
    fn the_reviewer_bots_status_is_no_ci_result() {
        let listing = [listed("instrukt/review", "failure", (3, "BOT_3"))];
        let (status, review) = reduce(&listing, &reviewer());
        assert_eq!(review.as_deref(), Some("failure"));
        assert_eq!(classify(&status, &checks(&[])), CiState::NothingReported);
    }

    #[test]
    fn without_the_reviewer_bots_status_the_ci_statuses_classify_as_before() {
        let listing = [listed("ci", "success", (9, "U_9"))];
        let (status, review) = reduce(&listing, &reviewer());
        assert_eq!(review, None);
        assert_eq!(classify(&status, &checks(&[])), CiState::Green);
    }

    #[test]
    fn a_context_that_failed_and_then_succeeded_on_a_rerun_is_green() {
        let newest_first = [
            listed("ci", "success", (9, "U_9")),
            listed("ci", "failure", (9, "U_9")),
        ];
        let (status, _) = reduce(&newest_first, &reviewer());
        assert_eq!(classify(&status, &checks(&[])), CiState::Green);
    }

    #[test]
    fn another_identitys_status_of_the_review_name_is_read_like_any_other() {
        let listing = [
            listed("instrukt/review", "failure", (9, "U_9")),
            listed("instrukt/review", "success", (3, "BOT_3")),
        ];
        let (status, review) = reduce(&listing, &reviewer());
        assert_eq!(review.as_deref(), Some("success"));
        assert_eq!(classify(&status, &checks(&[])), CiState::Red);
    }

    fn sha(c: char) -> CommitSha {
        CommitSha::parse(&c.to_string().repeat(40)).unwrap()
    }

    #[test]
    fn a_red_canonical_head_is_reported_as_failing_and_nothing_else_is() {
        let project = ProjectSlug::parse("demo").unwrap();
        let head = sha('a');
        assert_eq!(
            events(&project, None, None, None, Some(&head), Some(CiState::Red)),
            [Event::MainFailing {
                project: project.clone(),
                head: head.clone()
            }]
        );
        for state in [CiState::Green, CiState::Pending, CiState::NothingReported] {
            assert!(events(&project, None, None, None, Some(&head), Some(state)).is_empty());
        }
        let candidate = sha('b');
        assert_eq!(
            events(
                &project,
                Some(&candidate),
                Some(CiState::Green),
                None,
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
