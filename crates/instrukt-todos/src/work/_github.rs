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
    #[error("GitHub's answer to {path} is not readable: {message}")]
    Decode { path: String, message: String },
}

impl GitHubError {
    /// Whether the failure stops every further request of this command:
    /// GitHub cannot be reached, deferred us, or refused the token.
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
    started: Instant,
    apps: Option<std::sync::Arc<super::GitHubAccess>>,
    bots: std::cell::RefCell<Vec<instrukt_core::github::Bot>>,
    now: Timestamp,
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
            started: Instant::now(),
            apps: None,
            bots: Default::default(),
            now: Timestamp::now(),
        }
    }

    pub fn for_app(
        access: std::sync::Arc<super::GitHubAccess>,
        api: &str,
        repository: &str,
        role: Option<&str>,
        operation: instrukt_core::github::Operation,
        now: Timestamp,
    ) -> Result<Self, GitHubError> {
        access.validate()?;
        let apps = access;
        let mut gh = Self::new(api, repository, String::new());
        gh.apps = Some(apps);
        gh.now = now;
        gh.role = role.map(str::to_owned);
        gh.operation = operation;
        Ok(gh)
    }
    pub fn for_role(
        &self,
        role: Option<&str>,
        operation: instrukt_core::github::Operation,
    ) -> Result<Self, GitHubError> {
        let mut gh = Self::new(&self.api, &self.repository, String::new());
        gh.apps = self.apps.clone();
        gh.now = self.current_time()?;
        gh.role = role.map(str::to_owned);
        gh.operation = operation;
        Ok(gh)
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
    fn current_time(&self) -> Result<Timestamp, GitHubError> {
        let elapsed = i64::try_from(self.started.elapsed().as_secs()).unwrap_or(i64::MAX);
        Timestamp::from_second(self.now.as_second().saturating_add(elapsed)).map_err(|_| {
            GitHubError::Decode {
                path: "App".into(),
                message: "credential clock is out of range".into(),
            }
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
            let credential =
                apps.issue(&self.repository, self.role.as_deref(), self.operation, now)?;
            let bots = apps.verified_bots(&self.repository, now)?;
            *self.token.borrow_mut() = credential.token().to_owned();
            *self.credential.borrow_mut() = Some(credential);
            *self.bots.borrow_mut() = bots;
        }
        Ok(())
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
    /// them.
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
        check(
            status,
            &response,
            "POST",
            "/graphql",
            &answer.replace(self.token.borrow().as_str(), "<credential>"),
        )?;
        let value = decode("/graphql", &answer)?;
        if let Some(errors) = value["errors"].as_array().filter(|e| !e.is_empty()) {
            let message = errors
                .iter()
                .filter_map(|e| e["message"].as_str())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(GitHubError::Status {
                method: "POST",
                path: "/graphql".to_owned(),
                status,
                message: message.replace(self.token.borrow().as_str(), "<credential>"),
            });
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
    method: &'static str,
    path: &str,
    text: &str,
) -> Result<(), GitHubError> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    let remaining = header(response, "x-ratelimit-remaining");
    let retry_after = header(response, "retry-after");
    if status == 429
        || (status == 403 && (remaining.as_deref() == Some("0") || retry_after.is_some()))
    {
        let until = header(response, "x-ratelimit-reset")
            .and_then(|reset| reset.parse::<i64>().ok())
            .and_then(|secs| Timestamp::from_second(secs).ok());
        return Err(GitHubError::RateLimited { until });
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
