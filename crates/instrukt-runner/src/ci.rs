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
use std::path::Path;
use std::time::Duration;

use instrukt_core::config::{Configuration, Env};
use instrukt_core::git;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::events::CiResult;
use crate::process::Invocation;
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
        let status_body = self.get(&format!("{base}/status"), &mut validators, &mut interval)?;
        let checks_body = self.get(
            &format!("{base}/check-runs"),
            &mut validators,
            &mut interval,
        )?;
        validators.not_before =
            interval.and_then(|secs: i64| now.checked_add(SignedDuration::from_secs(secs)).ok());
        state::write_json(&path, &validators)?;
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

/// The GitHub token: `GITHUB_TOKEN`, `GH_TOKEN`, then `gh auth token`;
/// never stored.
pub fn token(env: &Env) -> Option<String> {
    for name in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Some(value) = env.get(name).filter(|v| !v.trim().is_empty()) {
            return Some(value.trim().to_owned());
        }
    }
    match Invocation::program("gh", &["auth", "token"]).run(Duration::from_secs(10)) {
        Ok(done) if done.success && !done.stdout.trim().is_empty() => {
            Some(done.stdout.trim().to_owned())
        }
        Ok(_) => None,
        Err(err) => {
            tracing::debug!(event = "github_token_unavailable", error = %err);
            None
        }
    }
}

/// The project's GitHub repository (`owner/repo`), resolved as work
/// resolves it (`github-record`, "Writer and convergence"): the
/// project-layer key `github.repository`, else the repository of the
/// remote the default branch tracks, when that remote is on
/// `github.api_url`'s host. The default branch is the remotes' `HEAD`, then
/// `init.defaultBranch`, then `main`. The reason is returned when none
/// resolves.
pub fn repository(config: &Configuration, root: &Path) -> Result<String, String> {
    if let Some(configured) = config.github_repository().map_err(|e| e.to_string())? {
        return owner_repo(configured)
            .ok_or_else(|| format!("`github.repository: {configured}` is not `<owner>/<repo>`"));
    }
    let host = api_host(config.github_api_url().map_err(|e| e.to_string())?);
    let branch = default_branch(root)?;
    let remote = git_value(root, &["config", &format!("branch.{branch}.remote")])?
        .ok_or_else(|| format!("the default branch `{branch}` tracks no remote"))?;
    let url = git_value(root, &["remote", "get-url", &remote])?
        .ok_or_else(|| format!("the remote `{remote}` has no URL"))?;
    parse_remote_url(&url, &host)
        .ok_or_else(|| format!("the remote `{remote}` is not a GitHub repository on {host}"))
}

fn default_branch(root: &Path) -> Result<String, String> {
    let remotes = git_value(root, &["remote"])?.unwrap_or_default();
    for remote in remotes.lines().map(str::trim).filter(|r| !r.is_empty()) {
        let head = format!("refs/remotes/{remote}/HEAD");
        if let Some(target) = git_value(root, &["symbolic-ref", "--quiet", &head])?
            && let Some(branch) = target.strip_prefix(&format!("refs/remotes/{remote}/"))
        {
            return Ok(branch.to_owned());
        }
    }
    Ok(git_value(root, &["config", "init.defaultBranch"])?.unwrap_or_else(|| "main".to_owned()))
}

/// A git query whose "no answer" is a non-zero exit with no output.
fn git_value(root: &Path, args: &[&str]) -> Result<Option<String>, String> {
    match git::run(root, args) {
        Ok(text) => {
            let text = text.trim();
            Ok((!text.is_empty()).then(|| text.to_owned()))
        }
        Err(git::GitError::Failed { stderr, .. }) if stderr.trim().is_empty() => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

/// The host whose repositories the API at `api_url` serves:
/// `api.github.com` serves `github.com`; any other address its own host.
pub fn api_host(api_url: &str) -> String {
    let rest = api_url.split_once("://").map_or(api_url, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or(rest);
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    match host.as_str() {
        "api.github.com" => "github.com".to_owned(),
        _ => host,
    }
}

/// `owner/repo` of a remote URL on `host`.
pub fn parse_remote_url(url: &str, host: &str) -> Option<String> {
    let path = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let authority = authority.rsplit('@').next()?;
        let remote_host = authority.split(':').next()?;
        if !remote_host.eq_ignore_ascii_case(host) {
            return None;
        }
        path
    } else {
        let (authority, path) = url.split_once(':')?;
        let remote_host = authority.rsplit('@').next()?;
        if !remote_host.eq_ignore_ascii_case(host) {
            return None;
        }
        path
    };
    owner_repo(path.trim_end_matches('/').trim_end_matches(".git"))
}

fn owner_repo(text: &str) -> Option<String> {
    let (owner, repo) = text.split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (valid(owner) && valid(repo)).then(|| format!("{owner}/{repo}"))
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

    #[test]
    fn remotes_on_the_api_host_name_their_repository() {
        assert_eq!(api_host("https://api.github.com"), "github.com");
        assert_eq!(api_host("https://ghe.example/api/v3"), "ghe.example");
        for url in [
            "git@github.com:acme/tool.git",
            "https://github.com/acme/tool",
            "ssh://git@github.com/acme/tool.git",
            "https://token@github.com/acme/tool.git/",
        ] {
            assert_eq!(
                parse_remote_url(url, "github.com"),
                owner_repo("acme/tool"),
                "{url}"
            );
        }
        for url in [
            "/srv/git/tool.git",
            "git@gitlab.com:acme/tool.git",
            "https://github.com/acme",
            "https://github.com/acme/../x",
        ] {
            assert_eq!(parse_remote_url(url, "github.com"), None, "{url}");
        }
    }
}
