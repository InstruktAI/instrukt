//! Candidate CI results from GitHub (`integration-gate`, "Candidate and
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
use serde::{Deserialize, Serialize};

use crate::events::CiResult;
use crate::state::{self, RunnerState, StateError};

/// How long one GitHub request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A commit's CI state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiState {
    Green,
    Red,
    Pending,
}

impl CiState {
    /// The completed result an event carries; `None` while pending.
    pub fn completed(self) -> Option<CiResult> {
        match self {
            CiState::Green => Some(CiResult::Success),
            CiState::Red => Some(CiResult::Failure),
            CiState::Pending => None,
        }
    }
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
    #[serde(default)]
    pub check_runs: Vec<CheckRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CheckRun {
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
    let check_failed = checks.check_runs.iter().any(|run| {
        run.status == "completed"
            && !matches!(
                run.conclusion.as_deref(),
                Some("success" | "neutral" | "skipped")
            )
    });
    if status_failed || check_failed {
        return CiState::Red;
    }
    if status.statuses.is_empty() && checks.check_runs.is_empty() {
        return CiState::Pending;
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
    State(CiState),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Validators {
    #[serde(default)]
    answers: BTreeMap<String, Cached>,
    #[serde(default)]
    not_before: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cached {
    etag: String,
    body: String,
}

/// GitHub's commit status API for one project's repository.
pub struct CiClient<'a> {
    api_url: String,
    repository: String,
    token: Option<String>,
    state: &'a RunnerState,
    project: &'a ProjectSlug,
}

impl<'a> CiClient<'a> {
    pub fn new(
        api_url: &str,
        repository: &str,
        token: Option<String>,
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

    /// Reads `commit`'s CI state at `now`, unless the source deferred.
    pub fn read(&self, commit: &CommitSha, now: Timestamp) -> Result<CiRead, CiError> {
        let path = self.state.validators(self.project);
        let mut validators: Validators = state::read_json(&path)?.unwrap_or_default();
        if let Some(not_before) = validators.not_before
            && now < not_before
        {
            return Ok(CiRead::Deferred(not_before));
        }
        let base = format!(
            "{}/repos/{}/commits/{commit}",
            self.api_url, self.repository
        );
        let mut interval = None;
        let bodies = self
            .get(&format!("{base}/status"), &mut validators, &mut interval)
            .and_then(|status| {
                self.get(
                    &format!("{base}/check-runs"),
                    &mut validators,
                    &mut interval,
                )
                .map(|checks| (status, checks))
            });
        // A source's poll interval binds whether or not its answer was
        // usable: an error carrying `X-Poll-Interval` still defers the
        // next request.
        validators.not_before =
            interval.and_then(|secs: i64| now.checked_add(SignedDuration::from_secs(secs)).ok());
        state::write_json(&path, &validators)?;
        let (status_body, checks_body) = bodies?;
        let status: CombinedStatus = parse(&format!("{base}/status"), &status_body)?;
        let checks: CheckRuns = parse(&format!("{base}/check-runs"), &checks_body)?;
        Ok(CiRead::State(classify(&status, &checks)))
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
        if let Some(token) = &self.token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }
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
            check_runs: runs
                .iter()
                .map(|(status, conclusion)| CheckRun {
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
            CiState::Pending
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

    /// A server answering every request with 503 and a poll interval,
    /// counting the requests it received.
    fn unavailable_server() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{BufRead as _, BufReader, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = std::sync::Arc::clone(&count);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                    line.clear();
                }
                seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let _ = stream.write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nX-Poll-Interval: 3600\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        (url, count)
    }

    #[test]
    fn a_failed_answer_still_defers_the_next_poll() {
        let dir = tempfile::tempdir().unwrap();
        let state = RunnerState::new(&instrukt_core::home::Home::at_user_home(dir.path().into()));
        let project = ProjectSlug::parse("demo").unwrap();
        let (url, requests) = unavailable_server();
        let client = CiClient::new(&url, "acme/demo", None, &state, &project);
        let commit = CommitSha::parse(&"a".repeat(40)).unwrap();
        let now: Timestamp = "2030-01-06T09:00:00Z".parse().unwrap();

        assert!(matches!(
            client.read(&commit, now),
            Err(CiError::Status { status: 503, .. })
        ));
        let later = now.checked_add(SignedDuration::from_secs(60)).unwrap();
        assert!(matches!(
            client.read(&commit, later),
            Ok(CiRead::Deferred(_))
        ));
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
