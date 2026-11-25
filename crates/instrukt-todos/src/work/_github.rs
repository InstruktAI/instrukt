//! GitHub's REST API as the work record reaches it: one repository, one
//! token, bounded waits. Translates transport failures, rate limiting and
//! error statuses into [`GitHubError`]; everything above speaks JSON.

use std::time::{Duration, Instant};

use jiff::Timestamp;
use serde_json::Value;

/// How long one request may take in total.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long connecting may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The page size of list reads.
const PAGE_SIZE: usize = 100;
/// The most pages one list read follows.
const MAX_PAGES: usize = 50;
const API_VERSION: &str = "2026-03-10";

/// A GitHub request did not succeed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitHubError {
    #[error("{message}")]
    RecordSchema { message: String },
    #[error(transparent)]
    App(#[from] instrukt_core::github::AppError),
    /// The App one todo's operation selected (a role's pull request or
    /// review App, or the board account) cannot serve it: its credential
    /// could not be issued because of a setup or identity mismatch or a
    /// refusal of that App's request, or the reviewer App's verified bot is
    /// the pull request's author. Not a reach failure; the call site
    /// decides what it stops ([`GitHubError::is_reach`]).
    #[error(transparent)]
    OperationApp(OperationAppError),
    #[error("GitHub is unreachable ({0})")]
    Unreachable(String),
    #[error("GitHub deferred requests (rate limited)")]
    RateLimited { until: Option<Timestamp> },
    #[error("GitHub refused the credentials ({status})")]
    Unauthorized { status: u16 },
    #[error("GitHub has no {path}")]
    NotFound { path: String },
    #[error("GitHub answered {method} {path} with {status}: {message}")]
    Status {
        method: &'static str,
        path: String,
        status: u16,
        message: String,
    },
    /// GitHub will not perform this operation for this viewer: a GraphQL
    /// `errors` answer whose every entry is typed `FORBIDDEN`. Unlike
    /// [`GitHubError::Unauthorized`], it says nothing about the credential's
    /// other uses.
    #[error("GitHub answered POST /graphql with {status}: {message}")]
    Refused { status: u16, message: String },
    #[error("GitHub's answer to {path} is not readable: {message}")]
    Decode { path: String, message: String },
}

/// Why one todo's operation-selected App cannot serve its operation: a
/// setup or identity mismatch or a refusal of that App's issuance request,
/// or a verified reviewer bot that is also the pull request's author. Only
/// [`GitHub::app_failure`] makes one, so it never holds an unreachable,
/// throttled or unavailable App API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(transparent)]
pub struct OperationAppError(instrukt_core::github::AppError);

impl From<OperationAppError> for instrukt_core::github::AppError {
    fn from(error: OperationAppError) -> Self {
        error.0
    }
}

impl GitHubError {
    /// Whether the failure concerns GitHub's reach rather than one
    /// request: GitHub cannot be reached, deferred us, or refused the token,
    /// or an installation credential could not be issued for a reason other
    /// than one todo's operation-selected App ([`GitHubError::OperationApp`]).
    /// This classifies; each caller decides the effect. A reach failure of a
    /// todo's record, issue, pull request or review reflection stops the
    /// command's remaining GitHub requests. Board setup stops them only on a
    /// rate limit, and a todo's board update only on a rate limit or an
    /// unreachable GitHub; any other board failure stays with the board.
    pub(crate) fn is_reach(&self) -> bool {
        matches!(
            self,
            GitHubError::Unreachable(_)
                | GitHubError::App(_)
                | GitHubError::RateLimited { .. }
                | GitHubError::Unauthorized { .. }
        )
    }
}

/// A conditional read's answer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Fetched {
    NotModified,
    Fresh { body: Value, etag: Option<String> },
}

/// A client for one repository.
pub(crate) struct GitHub {
    agent: ureq::Agent,
    api: String,
    repository: String,
    token: std::cell::RefCell<String>,
    credential: std::cell::RefCell<Option<std::sync::Arc<instrukt_core::github::Credential>>>,
    role: Option<String>,
    operation: instrukt_core::github::Operation,
    /// The pass's clock: requests, derived clients and issuance read the
    /// present from its one origin.
    clock: super::_apps::Clock,
    apps: Option<std::sync::Arc<super::GitHubAccess>>,
    /// Whether this client's credential serves one operation rather than
    /// every request of the command ([`GitHub::for_role`]).
    scoped: bool,
    bots: std::cell::RefCell<Vec<instrukt_core::github::Bot>>,
}

impl GitHub {
    pub fn new(api_url: &str, repository: &str, token: String) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .user_agent("instrukt")
            // The token goes only to the configured API host: a redirect
            // elsewhere (or to a weaker scheme) drops `Authorization`.
            .redirect_auth_headers(ureq::config::RedirectAuthHeaders::SameHost)
            .build()
            .into();
        Self {
            agent,
            api: api_url.trim_end_matches('/').to_owned(),
            repository: repository.to_owned(),
            token: std::cell::RefCell::new(token),
            credential: Default::default(),
            role: None,
            operation: instrukt_core::github::Operation::Record,
            clock: super::_apps::Clock::starting(Timestamp::now()),
            apps: None,
            scoped: false,
            bots: Default::default(),
        }
    }

    /// A client of `access` on `clock`, anchored where the caller sampled its
    /// `now`, so the wait for the capability lock here counts as elapsed.
    pub fn for_app(
        access: std::sync::Arc<super::GitHubAccess>,
        api: &str,
        repository: &str,
        role: Option<&str>,
        operation: instrukt_core::github::Operation,
        clock: super::_apps::Clock,
    ) -> Result<Self, GitHubError> {
        access.validate()?;
        let apps = access;
        let mut gh = Self::new(api, repository, String::new());
        gh.apps = Some(apps);
        gh.clock = clock;
        gh.role = role.map(str::to_owned);
        gh.operation = operation;
        Ok(gh)
    }
    pub fn for_role(
        &self,
        role: Option<&str>,
        operation: instrukt_core::github::Operation,
    ) -> Self {
        let mut gh = Self::new(&self.api, &self.repository, String::new());
        gh.apps = self.apps.clone();
        gh.scoped = true;
        gh.clock = self.clock;
        gh.role = role.map(str::to_owned);
        gh.operation = operation;
        gh
    }
    pub fn bots(&self) -> std::cell::Ref<'_, Vec<instrukt_core::github::Bot>> {
        self.bots.borrow()
    }
    pub fn trusts(&self, comment: &Value) -> bool {
        self.bots
            .borrow()
            .iter()
            .any(|bot| bot.matches(&comment["user"]))
    }
    pub fn bot(&self) -> Result<instrukt_core::github::Bot, GitHubError> {
        self.credential()?;
        self.credential
            .borrow()
            .as_ref()
            .map(|c| c.bot.clone())
            .ok_or_else(|| GitHubError::Decode {
                path: "App".into(),
                message: "verified bot unavailable".into(),
            })
    }
    /// The exact present moment on this client's clock.
    fn current_time(&self) -> Result<Timestamp, GitHubError> {
        self.clock.checked_now().ok_or_else(|| GitHubError::Decode {
            path: "App".into(),
            message: "credential clock is out of range".into(),
        })
    }
    fn credential(&self) -> Result<(), GitHubError> {
        let Some(apps) = &self.apps else {
            return Ok(());
        };
        let now = self.current_time()?;
        if self
            .credential
            .borrow()
            .as_ref()
            .is_none_or(|c| c.expires.as_second() <= now.as_second() + 30)
        {
            let credential = apps
                .issue(
                    &self.repository,
                    self.role.as_deref(),
                    self.operation,
                    self.clock,
                )
                .map_err(|error| self.app_failure(error))?;
            let bots = apps.verified_bots(&self.repository, self.clock)?;
            *self.token.borrow_mut() = credential.token().to_owned();
            *self.credential.borrow_mut() = Some(credential);
            *self.bots.borrow_mut() = bots;
        }
        Ok(())
    }

    /// An App failure as this client's scope makes it: an issuance failure,
    /// or the reviewer App's verified bot authoring the pull request. For an
    /// App one operation selected, a setup mismatch or a refusal of that
    /// App's request (a 4xx other than 403, which may be a secondary rate
    /// limit) is [`GitHubError::OperationApp`]. Everything else, and any
    /// failure of the shared project credential, is [`GitHubError::App`], a
    /// reach failure.
    pub(crate) fn app_failure(&self, error: instrukt_core::github::AppError) -> GitHubError {
        use instrukt_core::github::AppError;
        let local = match &error {
            AppError::Setup(_) => true,
            AppError::Refused { status, .. } => (400..500).contains(status) && *status != 403,
            AppError::Unreachable | AppError::RateLimited { .. } => false,
        };
        if local && self.scoped {
            GitHubError::OperationApp(OperationAppError(error))
        } else {
            GitHubError::App(error)
        }
    }

    pub fn can_write(&self, login: &str) -> Result<bool, GitHubError> {
        let path = format!("/collaborators/{login}/permission");
        let permission = self.read(&path)?;
        if let Some(push) = permission["user"]["permissions"]["push"].as_bool() {
            return Ok(push);
        }
        match permission["permission"].as_str() {
            Some("write" | "maintain" | "admin") => Ok(true),
            Some("read" | "none" | "triage" | "pull") => Ok(false),
            _ => Err(GitHubError::Decode {
                path,
                message: "missing or unrecognized repository permission".into(),
            }),
        }
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// `GET /repos/{repository}{path}`, conditional on `etag`.
    pub fn get(&self, path: &str, etag: Option<&str>) -> Result<Fetched, GitHubError> {
        let url = self.url(path);
        let mut request = self.with_headers(self.agent.get(&url))?;
        if let Some(etag) = etag {
            request = request.header("If-None-Match", etag);
        }
        let started = Instant::now();
        let mut response = request.call().map_err(transport)?;
        let status = response.status().as_u16();
        log(status, "GET", path, started);
        if status == 304 {
            return Ok(Fetched::NotModified);
        }
        let etag = header(&response, "etag");
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|err| GitHubError::Unreachable(err.to_string()))?;
        check(
            status,
            &response,
            self.current_time()?,
            "GET",
            path,
            &text.replace(self.token.borrow().as_str(), "<credential>"),
        )?;
        Ok(Fetched::Fresh {
            body: decode(path, &text)?,
            etag,
        })
    }

    /// `GET` of a resource that must answer fresh.
    pub fn read(&self, path: &str) -> Result<Value, GitHubError> {
        match self.get(path, None)? {
            Fetched::Fresh { body, .. } => Ok(body),
            Fetched::NotModified => Err(GitHubError::Decode {
                path: path.to_owned(),
                message: "unexpected 304 to an unconditional read".to_owned(),
            }),
        }
    }

    /// Every item of a list endpoint, following pages.
    pub fn list(&self, path: &str) -> Result<Vec<Value>, GitHubError> {
        let separator = if path.contains('?') { '&' } else { '?' };
        let mut items = Vec::new();
        for page in 1..=MAX_PAGES {
            let paged = format!("{path}{separator}per_page={PAGE_SIZE}&page={page}");
            let body = self.read(&paged)?;
            let Value::Array(batch) = body else {
                return Err(GitHubError::Decode {
                    path: paged,
                    message: "expected a list".to_owned(),
                });
            };
            let full = batch.len() == PAGE_SIZE;
            items.extend(batch);
            if !full {
                break;
            }
        }
        Ok(items)
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value, GitHubError> {
        self.send("POST", path, Some(body))
    }

    pub fn patch(&self, path: &str, body: &Value) -> Result<Value, GitHubError> {
        self.send("PATCH", path, Some(body))
    }

    fn send(
        &self,
        method: &'static str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, GitHubError> {
        self.send_to(method, path, &self.url(path), body)
    }

    /// Account-scoped REST operation on the configured API origin.
    pub fn post_api(&self, path: &str, body: &Value) -> Result<Value, GitHubError> {
        self.send_to("POST", path, &format!("{}{path}", self.api), Some(body))
    }

    fn send_to(
        &self,
        method: &'static str,
        path: &str,
        url: &str,
        body: Option<&Value>,
    ) -> Result<Value, GitHubError> {
        let text = body.map(Value::to_string).unwrap_or_default();
        let started = Instant::now();
        let result = match method {
            "POST" => self
                .with_headers(self.agent.post(url))?
                .header("Content-Type", "application/json")
                .send(&text),
            _ => self
                .with_headers(self.agent.patch(url))?
                .header("Content-Type", "application/json")
                .send(&text),
        };
        let mut response = result.map_err(transport)?;
        let status = response.status().as_u16();
        log(status, method, path, started);
        let answer = response
            .body_mut()
            .read_to_string()
            .map_err(|err| GitHubError::Unreachable(err.to_string()))?;
        check(
            status,
            &response,
            self.current_time()?,
            method,
            path,
            &answer.replace(self.token.borrow().as_str(), "<credential>"),
        )?;
        if answer.trim().is_empty() {
            return Ok(Value::Null);
        }
        decode(path, &answer)
    }

    /// A GraphQL document with its variables (never interpolated values);
    /// the `data` of the answer. An `errors` answer is a failure naming
    /// them, typed by [`graphql_failure`].
    pub fn graphql(&self, document: &str, variables: &Value) -> Result<Value, GitHubError> {
        let url = self.graphql_url();
        let text = serde_json::json!({ "query": document, "variables": variables }).to_string();
        let started = Instant::now();
        let mut response = self
            .with_headers(self.agent.post(&url))?
            .header("Content-Type", "application/json")
            .send(&text)
            .map_err(transport)?;
        let status = response.status().as_u16();
        log(status, "POST", "/graphql", started);
        let answer = response
            .body_mut()
            .read_to_string()
            .map_err(|err| GitHubError::Unreachable(err.to_string()))?;
        let now = self.current_time()?;
        check(
            status,
            &response,
            now,
            "POST",
            "/graphql",
            &answer.replace(self.token.borrow().as_str(), "<credential>"),
        )?;
        let value = decode("/graphql", &answer)?;
        if let Some(errors) = value["errors"].as_array().filter(|e| !e.is_empty()) {
            return Err(graphql_failure(
                errors,
                status,
                &Limits::read(response.headers()),
                now,
                self.token.borrow().as_str(),
            ));
        }
        Ok(value["data"].clone())
    }

    /// GitHub's GraphQL endpoint beside the REST address: `/graphql` of
    /// the public API, `/api/graphql` beside an enterprise `/api/v3`.
    fn graphql_url(&self) -> String {
        match self.api.strip_suffix("/api/v3") {
            Some(host) => format!("{host}/api/graphql"),
            None => format!("{}/graphql", self.api),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/repos/{}{}", self.api, self.repository, path)
    }

    fn with_headers<B>(
        &self,
        request: ureq::RequestBuilder<B>,
    ) -> Result<ureq::RequestBuilder<B>, GitHubError> {
        self.credential()?;
        Ok(request
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
            .header("Authorization", &format!("Bearer {}", self.token.borrow())))
    }
}

fn transport(err: ureq::Error) -> GitHubError {
    GitHubError::Unreachable(err.to_string())
}

fn header(response: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn check(
    status: u16,
    response: &ureq::http::Response<ureq::Body>,
    now: Timestamp,
    method: &'static str,
    path: &str,
    text: &str,
) -> Result<(), GitHubError> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    let limits = Limits::read(response.headers());
    if limits.refusal_throttles(status) {
        return Err(GitHubError::RateLimited {
            until: limits.until(now),
        });
    }
    if status == 401 {
        return Err(GitHubError::Unauthorized { status });
    }
    if status == 404 {
        return Err(GitHubError::NotFound {
            path: path.to_owned(),
        });
    }
    let message = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value["message"].as_str().map(str::to_owned))
        .unwrap_or_default();
    Err(GitHubError::Status {
        method,
        path: path.to_owned(),
        status,
        message,
    })
}

/// GitHub's rate-limit signals on one answer: the primary quota's
/// `x-ratelimit-remaining` and `x-ratelimit-reset`, and the `retry-after`
/// delay a secondary limit sends. Every GitHub answer Work reads, REST or
/// GraphQL, through the record client or the App issuer, is judged by them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Limits {
    remaining: Option<String>,
    reset: Option<String>,
    retry_after: Option<String>,
}

impl Limits {
    pub(super) fn read(headers: &ureq::http::HeaderMap) -> Self {
        let get = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        Self {
            remaining: get("x-ratelimit-remaining"),
            reset: get("x-ratelimit-reset"),
            retry_after: get("retry-after"),
        }
    }

    /// Whether GitHub signalled throttling: no requests remain, or it named
    /// a delay before the next one.
    fn signalled(&self) -> bool {
        self.remaining.as_deref() == Some("0") || self.retry_after.is_some()
    }

    /// Whether an unsuccessful `status` is throttling: 429, or a 403 that
    /// carries a throttle signal (otherwise a 403 is a refusal).
    pub(super) fn refusal_throttles(&self, status: u16) -> bool {
        status == 429 || (status == 403 && self.signalled())
    }

    /// The earliest moment GitHub permits the next request, for an answer
    /// received at the exact moment `received`: not before `retry-after`
    /// seconds have passed, nor before the reset when no requests remain;
    /// without a delay, the reset GitHub sent. A delay counts whole seconds
    /// from `received` rounded up, so it never ends before the full delay
    /// has passed; the reset is GitHub's own instant and stays exact.
    pub(super) fn until(&self, received: Timestamp) -> Option<Timestamp> {
        self.deadline(received, None)
    }

    /// [`Limits::until`] for a secondary limit: one that names no delay
    /// waits GitHub's minute instead of the primary reset, unless no
    /// requests remain, when the later reset still governs.
    fn secondary_until(&self, received: Timestamp) -> Option<Timestamp> {
        self.deadline(received, Some(SECONDARY_WAIT_SECONDS))
    }

    fn deadline(&self, received: Timestamp, unnamed_delay: Option<i64>) -> Option<Timestamp> {
        let whole = received.as_second() + i64::from(received.subsec_nanosecond() > 0);
        let delay = self
            .retry_after
            .as_deref()
            .and_then(|seconds| seconds.trim().parse::<i64>().ok())
            .filter(|seconds| *seconds >= 0)
            .or(unnamed_delay)
            .and_then(|seconds| whole.checked_add(seconds))
            .and_then(|second| Timestamp::from_second(second).ok());
        let reset = self
            .reset
            .as_deref()
            .and_then(|reset| reset.parse::<i64>().ok())
            .and_then(|secs| Timestamp::from_second(secs).ok());
        if self.remaining.as_deref() == Some("0") {
            delay.max(reset)
        } else {
            delay.or(reset)
        }
    }
}

/// How long GitHub asks a client to wait after a secondary rate limit that
/// names no `retry-after` delay while requests remain: at least a minute.
const SECONDARY_WAIT_SECONDS: i64 = 60;

/// Whether a GraphQL `errors` entry is GitHub's secondary rate limit. GitHub
/// identifies it by its message alone, with neither a type nor, always, a
/// header; this reads that provider wire signal as octokit's throttling
/// plugin does (`/\bsecondary rate\b/i`).
fn secondary_limit(entry: &Value) -> bool {
    entry["message"]
        .as_str()
        .is_some_and(|message| message.to_ascii_lowercase().contains("secondary rate"))
}

/// The failure a GraphQL answer with a non-empty `errors` list reports,
/// received at `now`. A secondary rate limit is [`GitHubError::RateLimited`]
/// for its delay, a minute when it names none, or until the later reset when
/// no requests remain; any other answer GitHub throttled (a [`Limits`]
/// signal, whatever the HTTP status, or an entry typed `RATE_LIMITED`) is
/// [`GitHubError::RateLimited`] until [`Limits::until`]; otherwise one whose
/// every entry is typed `FORBIDDEN` is [`GitHubError::Refused`]; any other
/// answer is [`GitHubError::Status`]. Messages carry `credential` redacted.
pub(super) fn graphql_failure(
    errors: &[Value],
    status: u16,
    limits: &Limits,
    now: Timestamp,
    credential: &str,
) -> GitHubError {
    let typed = |entry: &Value, kind: &str| entry["type"].as_str() == Some(kind);
    if errors.iter().any(secondary_limit) {
        return GitHubError::RateLimited {
            until: limits.secondary_until(now),
        };
    }
    if limits.signalled() || errors.iter().any(|e| typed(e, "RATE_LIMITED")) {
        return GitHubError::RateLimited {
            until: limits.until(now),
        };
    }
    let message = errors
        .iter()
        .filter_map(|e| e["message"].as_str())
        .collect::<Vec<_>>()
        .join("; ");
    let message = message.replace(credential, "<credential>");
    if errors.iter().all(|e| typed(e, "FORBIDDEN")) {
        return GitHubError::Refused { status, message };
    }
    GitHubError::Status {
        method: "POST",
        path: "/graphql".to_owned(),
        status,
        message,
    }
}

fn decode(path: &str, text: &str) -> Result<Value, GitHubError> {
    serde_json::from_str(text).map_err(|err| GitHubError::Decode {
        path: path.to_owned(),
        message: err.to_string(),
    })
}

fn log(status: u16, method: &str, path: &str, started: Instant) {
    tracing::debug!(
        event = "github_request",
        method,
        path,
        status,
        duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "GitHub request"
    );
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The moment the answers below are seen; the primary reset they carry
    /// is ten minutes later.
    const NOW: i64 = 1_767_225_000;
    const RESET: i64 = 1_767_225_600;

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(second).unwrap()
    }

    fn limits(remaining: Option<&str>, reset: Option<i64>, retry_after: Option<&str>) -> Limits {
        Limits {
            remaining: remaining.map(str::to_owned),
            reset: reset.map(|reset| reset.to_string()),
            retry_after: retry_after.map(str::to_owned),
        }
    }

    fn answered(errors: Value, limits: &Limits) -> GitHubError {
        answered_at(errors, limits, at(NOW))
    }

    fn answered_at(errors: Value, limits: &Limits, received: Timestamp) -> GitHubError {
        let errors = errors.as_array().cloned().unwrap_or_default();
        graphql_failure(&errors, 200, limits, received, "secret")
    }

    /// An answer received 1.6 seconds after `NOW`.
    fn within_a_second() -> Timestamp {
        at(NOW)
            .checked_add(jiff::SignedDuration::from_millis(1_600))
            .unwrap()
    }

    fn secondary() -> Value {
        json!([{"message": "You have exceeded a secondary rate limit."}])
    }

    fn failure(errors: Value, remaining: Option<&str>) -> GitHubError {
        answered(errors, &limits(remaining, Some(RESET), None))
    }

    #[test]
    fn an_ok_graphql_error_with_retry_after_defers_for_its_delay_though_quota_remains() {
        let limits = limits(Some("4999"), None, Some("120"));
        let error = answered_at(secondary(), &limits, within_a_second());
        // The delay counts from the receipt rounded up to its second.
        assert_eq!(
            error,
            GitHubError::RateLimited {
                until: Some(at(NOW + 2 + 120))
            }
        );
        assert!(error.is_reach());
    }

    #[test]
    fn an_ok_secondary_limit_naming_no_delay_waits_a_minute_not_the_primary_reset() {
        let limits = limits(Some("4999"), Some(RESET), None);
        let error = answered_at(secondary(), &limits, within_a_second());
        assert_eq!(
            error,
            GitHubError::RateLimited {
                until: Some(at(NOW + 2 + 60))
            }
        );
    }

    #[test]
    fn an_ok_secondary_limit_with_no_requests_remaining_waits_for_the_exact_later_reset() {
        let reset = GitHubError::RateLimited {
            until: Some(at(RESET)),
        };
        let unnamed = limits(Some("0"), Some(RESET), None);
        assert_eq!(answered_at(secondary(), &unnamed, within_a_second()), reset);
        let named = limits(Some("0"), Some(RESET), Some("120"));
        assert_eq!(answered_at(secondary(), &named, within_a_second()), reset);
    }

    #[test]
    fn a_typed_rate_limited_entry_keeps_retry_after_over_a_sooner_reset() {
        let typed = json!([{"type": "RATE_LIMITED", "message": "limit"}]);
        let error = answered(typed, &limits(Some("4999"), Some(NOW + 30), Some("120")));
        assert_eq!(
            error,
            GitHubError::RateLimited {
                until: Some(at(NOW + 120))
            }
        );
    }

    #[test]
    fn with_no_requests_remaining_the_later_of_reset_and_retry_after_governs() {
        let typed = json!([{"type": "RATE_LIMITED", "message": "limit"}]);
        let error = answered(typed, &limits(Some("0"), Some(RESET), Some("120")));
        assert_eq!(
            error,
            GitHubError::RateLimited {
                until: Some(at(RESET))
            }
        );
    }

    #[test]
    fn an_entry_typed_rate_limited_defers_until_the_reset() {
        let error = failure(
            json!([{"type": "RATE_LIMITED", "message": "limit"}]),
            Some("4999"),
        );
        assert_eq!(
            error,
            GitHubError::RateLimited {
                until: Some("2026-01-01T00:00:00Z".parse().unwrap())
            }
        );
    }

    #[test]
    fn no_remaining_requests_defer_even_when_every_entry_is_forbidden() {
        let error = failure(json!([{"type": "FORBIDDEN", "message": "no"}]), Some("0"));
        assert!(matches!(error, GitHubError::RateLimited { until: Some(_) }));
    }

    #[test]
    fn an_answer_whose_every_entry_is_forbidden_is_a_refusal_not_a_reach_failure() {
        let error = failure(
            json!([
                {"type": "FORBIDDEN", "message": "a"},
                {"type": "FORBIDDEN", "message": "b"}
            ]),
            Some("4999"),
        );
        assert!(matches!(error, GitHubError::Refused { status: 200, .. }));
        assert!(!error.is_reach());
    }

    #[test]
    fn mixed_entry_types_stay_an_ordinary_status() {
        let error = failure(
            json!([{"type": "FORBIDDEN", "message": "a"}, {"type": "NOT_FOUND", "message": "b"}]),
            None,
        );
        assert!(matches!(error, GitHubError::Status { status: 200, .. }));
    }

    #[test]
    fn an_untyped_entry_stays_an_ordinary_status() {
        let error = failure(json!([{"message": "no type"}]), None);
        assert!(matches!(error, GitHubError::Status { status: 200, .. }));
    }

    fn shared() -> GitHub {
        GitHub::new("http://127.0.0.1", "o/r", String::new())
    }

    fn scoped() -> GitHub {
        shared().for_role(Some("builder"), instrukt_core::github::Operation::Author)
    }

    fn refused(status: u16) -> instrukt_core::github::AppError {
        instrukt_core::github::AppError::Refused {
            operation: "/app/installations/2/access_tokens".into(),
            status,
        }
    }

    #[test]
    fn a_setup_mismatch_of_the_shared_credential_stops_the_command() {
        let error = shared().app_failure(instrukt_core::github::AppError::Setup("mismatch".into()));
        assert!(matches!(error, GitHubError::App(_)));
        assert!(error.is_reach());
    }

    #[test]
    fn a_setup_mismatch_of_an_operation_app_stays_with_its_todo() {
        let error = scoped().app_failure(instrukt_core::github::AppError::Setup("mismatch".into()));
        assert!(matches!(error, GitHubError::OperationApp(_)));
        assert!(!error.is_reach());
    }

    #[test]
    fn an_operation_app_refused_with_a_client_error_stays_with_its_todo() {
        let error = scoped().app_failure(refused(404));
        assert!(matches!(error, GitHubError::OperationApp(_)));
        assert!(!error.is_reach());
    }

    #[test]
    fn an_operation_app_refused_as_forbidden_stops_the_command_as_a_possible_throttle() {
        let error = scoped().app_failure(refused(403));
        assert!(matches!(error, GitHubError::App(_)));
        assert!(error.is_reach());
    }

    #[test]
    fn an_operation_app_refused_by_an_unavailable_app_api_stops_the_command() {
        let error = scoped().app_failure(refused(503));
        assert!(matches!(error, GitHubError::App(_)));
        assert!(error.is_reach());
    }

    #[test]
    fn an_operation_app_deferred_by_the_app_api_stops_the_command() {
        let error =
            scoped().app_failure(instrukt_core::github::AppError::RateLimited { until: None });
        assert!(matches!(error, GitHubError::App(_)));
        assert!(error.is_reach());
    }
}
