//! GitHub's REST and GraphQL API as the work record reaches it: one repository,
//! one token, backed by [`octocrab`] and [`instrukt_outside`].

use std::future::Future;
use std::str::FromStr;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use serde_json::Value;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SECONDARY_WAIT_SECONDS: i64 = 60;
const API_VERSION: &str = "2026-03-10";
const MAX_PAGES: usize = 500;

/// A GitHub request did not succeed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitHubError {
    #[error(transparent)]
    App(#[from] instrukt_core::github::AppError),
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
    #[error("GitHub refused {method} {path} as invalid ({status}): {detail}")]
    Invalid {
        method: &'static str,
        path: String,
        status: u16,
        detail: String,
    },
    #[error("GitHub answered {method} {path} with {status}: {message}")]
    Status {
        method: &'static str,
        path: String,
        status: u16,
        message: String,
    },
    #[error("GitHub answered POST /graphql with {status}: {message}")]
    Refused { status: u16, message: String },
    #[error("GitHub's answer to {path} is not readable: {message}")]
    Decode { path: String, message: String },
}

/// Why one todo's operation-selected App cannot serve its operation. Only
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
    /// Whether the failure concerns GitHub's reach rather than one request.
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

/// A client for one repository backed by [`octocrab::Octocrab`].
pub(crate) struct GitHub {
    client: std::cell::RefCell<Option<(String, octocrab::Octocrab)>>,
    api: String,
    repository: String,
    token: std::cell::RefCell<String>,
    credential: std::cell::RefCell<Option<std::sync::Arc<instrukt_core::github::Credential>>>,
    role: Option<String>,
    operation: instrukt_core::github::Operation,
    clock: super::_apps::Clock,
    apps: Option<std::sync::Arc<super::GitHubAccess>>,
    gate: instrukt_outside::Gate,
    scoped: bool,
}

fn default_gate(clock: super::_apps::Clock) -> instrukt_outside::Gate {
    instrukt_outside::Gate::new(
        std::sync::Arc::new(move || clock.checked_now().unwrap_or(Timestamp::MAX)),
        std::sync::Arc::new(Instant::now),
        instrukt_outside::Report::log(),
    )
}

impl GitHub {
    pub fn new(api_url: &str, repository: &str, token: String) -> Self {
        let clock = super::_apps::Clock::starting(Timestamp::now());
        Self {
            client: Default::default(),
            api: api_url.trim_end_matches('/').to_owned(),
            repository: repository.to_owned(),
            token: std::cell::RefCell::new(token),
            credential: Default::default(),
            role: None,
            operation: instrukt_core::github::Operation::Record,
            clock,
            apps: None,
            gate: default_gate(clock),
            scoped: false,
        }
    }

    pub(super) fn with_gate(mut self, gate: instrukt_outside::Gate) -> Self {
        self.gate = gate;
        self
    }

    pub fn for_app(
        access: std::sync::Arc<super::GitHubAccess>,
        api: &str,
        repository: &str,
        role: Option<&str>,
        operation: instrukt_core::github::Operation,
        clock: super::_apps::Clock,
    ) -> Result<Self, GitHubError> {
        let gate = access.gate()?;
        let mut gh = Self::new(api, repository, String::new());
        gh.apps = Some(access);
        gh.gate = gate;
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
        gh.gate = self.gate.clone();
        gh.scoped = true;
        gh.clock = self.clock;
        gh.role = role.map(str::to_owned);
        gh.operation = operation;
        gh
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
            *self.token.borrow_mut() = credential.token().to_owned();
            *self.credential.borrow_mut() = Some(credential);
        }
        Ok(())
    }

    /// Classifies an App failure by client scope: setup mismatches and non-403
    /// 4xx refusals on a scoped operation App stay with that todo
    /// ([`GitHubError::OperationApp`]); everything else is a reach failure.
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
        let crab = self.octocrab()?;
        let route = self.repo_route(path);
        let headers = etag.and_then(if_none_match_header);
        let started = Instant::now();
        let (status, etag, text) = self.call("GET", path, async move {
            let response = crab._get_with_headers(route, headers).await?;
            let status = response.status().as_u16();
            if status == 304 {
                return Ok((status, None, None));
            }
            let etag = extract_etag(&response);
            let response = octocrab::map_github_error(response).await?;
            Ok((status, etag, Some(crab.body_to_string(response).await?)))
        })?;
        log(status, "GET", path, started);
        match text {
            None => Ok(Fetched::NotModified),
            Some(text) => Ok(Fetched::Fresh {
                body: decode(path, &text)?,
                etag,
            }),
        }
    }

    /// `GET` of a resource that must answer fresh.
    pub fn read(&self, path: &str) -> Result<Value, GitHubError> {
        let crab = self.octocrab()?;
        let route = self.repo_route(path);
        let started = Instant::now();
        let body = self.call("GET", path, async move {
            crab.get::<Value, _, _>(route, None::<&()>).await
        })?;
        log(200, "GET", path, started);
        Ok(body)
    }

    /// Every item of a list endpoint, validating each `Link` pagination URL's
    /// scheme and authority against the configured API origin before calling
    /// [`octocrab::Octocrab::get_page`] so installation tokens can never be
    /// downgraded to plaintext HTTP or redirected off-origin.
    pub fn list(&self, path: &str) -> Result<Vec<Value>, GitHubError> {
        let crab = self.octocrab()?;
        let base_uri: http::Uri = self
            .api
            .parse()
            .map_err(|err: http::uri::InvalidUri| GitHubError::Unreachable(err.to_string()))?;
        let sep = if path.contains('?') { '&' } else { '?' };
        let route = format!("{}{sep}per_page=100", self.repo_route(path));
        let started = Instant::now();
        let mut page: octocrab::Page<Value> =
            self.call("GET", path, async { crab.get(&route, None::<&()>).await })?;
        let mut items = page.take_items();
        let mut pages = 1usize;
        while let Some(next) = page.next.take() {
            if !link_admitted(&base_uri, &next) {
                return Err(GitHubError::Decode {
                    path: path.to_owned(),
                    message: "GitHub pagination link changed scheme or origin".into(),
                });
            }
            if pages >= MAX_PAGES {
                return Err(GitHubError::Decode {
                    path: path.to_owned(),
                    message: format!("GitHub pagination exceeded {MAX_PAGES} pages"),
                });
            }
            pages += 1;
            let next_opt = Some(next);
            let Some(next_page) = self.call("GET", path, async {
                crab.get_page::<Value>(&next_opt).await
            })?
            else {
                break;
            };
            page = next_page;
            items.extend(page.take_items());
        }
        log(200, "GET", path, started);
        Ok(items)
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value, GitHubError> {
        self.send_to("POST", path, self.repo_route(path), Some(body))
    }

    pub fn patch(&self, path: &str, body: &Value) -> Result<Value, GitHubError> {
        self.send_to("PATCH", path, self.repo_route(path), Some(body))
    }

    /// Account-scoped REST operation on the configured API origin.
    pub fn post_api(&self, path: &str, body: &Value) -> Result<Value, GitHubError> {
        self.send_to("POST", path, path.to_owned(), Some(body))
    }

    fn send_to(
        &self,
        method: &'static str,
        path: &str,
        route: String,
        body: Option<&Value>,
    ) -> Result<Value, GitHubError> {
        let crab = self.octocrab()?;
        let payload = body.cloned();
        let started = Instant::now();
        let (status, answer) = self.call(method, path, async move {
            let response = match method {
                "POST" => crab._post(route, payload.as_ref()).await?,
                _ => crab._patch(route, payload.as_ref()).await?,
            };
            let status = response.status().as_u16();
            let response = octocrab::map_github_error(response).await?;
            Ok((status, crab.body_to_string(response).await?))
        })?;
        log(status, method, path, started);
        if answer.trim().is_empty() {
            return Ok(Value::Null);
        }
        decode(path, &answer)
    }

    /// A GraphQL document with its variables; returns `data` or a typed
    /// [`graphql_failure`] when `errors` is non-empty.
    pub fn graphql(&self, document: &str, variables: &Value) -> Result<Value, GitHubError> {
        let (crab, route) = match self.api.strip_suffix("/api/v3") {
            Some(host) => {
                self.credential()?;
                let token = self.token.borrow().clone();
                (build_octocrab(host, &token)?, "/api/graphql")
            }
            None => (self.octocrab()?, "/graphql"),
        };
        let payload = serde_json::json!({ "query": document, "variables": variables });
        let started = Instant::now();
        let (status, headers, answer) = self.call("POST", "/graphql", async move {
            let response = crab._post(route, Some(&payload)).await?;
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let response = octocrab::map_github_error(response).await?;
            Ok((status, headers, crab.body_to_string(response).await?))
        })?;
        log(status, "POST", "/graphql", started);
        let value = decode("/graphql", &answer)?;
        if let Some(errors) = value["errors"].as_array().filter(|e| !e.is_empty()) {
            let now = self.current_time()?;
            return Err(graphql_failure(
                errors,
                status,
                &Limits::read(&headers),
                now,
                self.token.borrow().as_str(),
            ));
        }
        Ok(value["data"].clone())
    }

    fn call<T, F>(&self, method: &'static str, path: &str, future: F) -> Result<T, GitHubError>
    where
        F: Future<Output = octocrab::Result<T>> + Send,
        T: Send,
    {
        let key = instrukt_outside::Key::new("github", [self.api.as_str()])
            .item(format!("{method} {path}"));
        let token = self.token.borrow().clone();
        let clock = self.clock;
        let gate = self.gate.clone();
        let attempted = instrukt_outside::runtime::block_on(gate.attempt_async(
            &key,
            &instrukt_outside::Settings::default(),
            || async move {
                let outcome = match tokio::time::timeout(REQUEST_TIMEOUT, future).await {
                    Ok(Ok(value)) => return instrukt_outside::Tried::Done(value),
                    Ok(Err(err)) => {
                        let now = clock.checked_now().unwrap_or(Timestamp::MAX);
                        map_octocrab_error(err, method, path, now, &token)
                    }
                    Err(_) => GitHubError::Unreachable("request timed out".to_owned()),
                };
                classify_for_gate(outcome)
            },
        ))
        .map_err(|err| GitHubError::Unreachable(err.to_string()))?;
        match attempted {
            instrukt_outside::Attempted::Done(value) => Ok(value),
            instrukt_outside::Attempted::Defect(err) => Err(err),
            instrukt_outside::Attempted::Wait {
                error: Some(err), ..
            } => Err(err),
            instrukt_outside::Attempted::Wait { wait, error: None } => {
                if wait.reason() == instrukt_outside::Reason::RateLimited {
                    Err(GitHubError::RateLimited {
                        until: Some(wait.at()),
                    })
                } else {
                    Err(GitHubError::Unreachable(wait.to_string()))
                }
            }
        }
    }

    fn repo_route(&self, path: &str) -> String {
        format!("/repos/{}{}", self.repository, path)
    }

    fn octocrab(&self) -> Result<octocrab::Octocrab, GitHubError> {
        self.credential()?;
        let token = self.token.borrow().clone();
        if let Some((cached_token, crab)) = self.client.borrow().as_ref()
            && cached_token == &token
        {
            return Ok(crab.clone());
        }
        let crab = build_octocrab(&self.api, &token)?;
        *self.client.borrow_mut() = Some((token, crab.clone()));
        Ok(crab)
    }
}

/// Whether a pagination `Link` URI preserves the configured API origin and
/// scheme (or is a path-only relative URI on that origin). Rejects scheme
/// downgrades (`https://` -> `http://`), scheme-relative URIs (`//host/...`),
/// and cross-origin URLs so installation tokens are never sent over plaintext
/// or to another host.
fn link_admitted(base: &http::Uri, next: &http::Uri) -> bool {
    match (next.scheme_str(), next.authority()) {
        (None, None) => next.path().starts_with('/') && !next.path().starts_with("//"),
        (Some(scheme), Some(authority)) => {
            base.scheme_str() == Some(scheme)
                && base.authority().map(http::uri::Authority::as_str) == Some(authority.as_str())
        }
        _ => false,
    }
}

fn classify_for_gate<T>(error: GitHubError) -> instrukt_outside::Tried<T, GitHubError> {
    match &error {
        GitHubError::RateLimited { until } => instrukt_outside::Tried::Unavailable(
            instrukt_outside::Unavailable {
                reason: instrukt_outside::Reason::RateLimited,
                extent: instrukt_outside::Extent::Key,
                named: until.map(instrukt_outside::Named::At),
            },
            error,
        ),
        GitHubError::Unreachable(msg) => instrukt_outside::Tried::Unavailable(
            instrukt_outside::Unavailable {
                reason: if msg.contains("timed out") {
                    instrukt_outside::Reason::TimedOut
                } else {
                    instrukt_outside::Reason::Unreachable
                },
                extent: instrukt_outside::Extent::Scope,
                named: None,
            },
            error,
        ),
        GitHubError::Status { status, .. } if *status >= 500 => {
            instrukt_outside::Tried::Unavailable(
                instrukt_outside::Unavailable {
                    reason: instrukt_outside::Reason::Unavailable {
                        status: Some(*status),
                    },
                    extent: instrukt_outside::Extent::Scope,
                    named: None,
                },
                error,
            )
        }
        _ => instrukt_outside::Tried::Defect(error),
    }
}

pub(super) fn octocrab_builder(
    base_uri: &str,
    api_version: Option<&str>,
) -> octocrab::Result<
    octocrab::OctocrabBuilder<
        octocrab::NoSvc,
        octocrab::DefaultOctocrabBuilderConfig,
        octocrab::NoAuth,
        octocrab::NotLayerReady,
    >,
> {
    let mut builder = octocrab::Octocrab::builder()
        .base_uri(base_uri)?
        .set_connect_timeout(Some(CONNECT_TIMEOUT))
        .set_read_timeout(Some(REQUEST_TIMEOUT))
        .set_write_timeout(Some(REQUEST_TIMEOUT))
        .add_header(http::header::USER_AGENT, "instrukt".to_owned())
        .add_header(
            http::header::ACCEPT,
            "application/vnd.github+json".to_owned(),
        );
    if let Some(version) = api_version {
        builder = builder.add_header(
            http::header::HeaderName::from_static("x-github-api-version"),
            version.to_owned(),
        );
    }
    Ok(builder)
}

fn build_octocrab(base_uri: &str, token: &str) -> Result<octocrab::Octocrab, GitHubError> {
    let rt = instrukt_outside::runtime::runtime()
        .map_err(|err| GitHubError::Unreachable(err.to_string()))?;
    let _guard = rt.enter();
    let mut builder = octocrab_builder(base_uri, Some(API_VERSION))
        .map_err(|err| GitHubError::Unreachable(err.to_string()))?;
    if !token.is_empty() {
        if http::HeaderValue::from_str(&format!("Bearer {token}")).is_err() {
            return Err(GitHubError::Unauthorized { status: 401 });
        }
        builder = builder.personal_token(token.to_owned());
    }
    builder
        .build()
        .map_err(|err| GitHubError::Unreachable(err.to_string()))
}

fn if_none_match_header(value: &str) -> Option<http::HeaderMap> {
    let mut map = http::HeaderMap::new();
    if let Ok(tag) = octocrab::etag::EntityTag::from_str(value)
        && octocrab::etag::EntityTag::insert_if_none_match_header(&mut map, tag).is_ok()
    {
        return Some(map);
    }
    map.insert(
        http::header::IF_NONE_MATCH,
        http::HeaderValue::from_str(value).ok()?,
    );
    Some(map)
}

fn extract_etag<B>(response: &http::Response<B>) -> Option<String> {
    octocrab::etag::EntityTag::extract_from_response(response)
        .map(|tag| tag.to_string())
        .or_else(|| {
            response
                .headers()
                .get(http::header::ETAG)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        })
}

fn map_octocrab_error(
    err: octocrab::Error,
    method: &'static str,
    path: &str,
    now: Timestamp,
    token: &str,
) -> GitHubError {
    match err {
        octocrab::Error::GitHub { source, .. } => {
            let status = source.status_code.as_u16();
            let empty = http::HeaderMap::new();
            let headers = source.headers.as_ref().unwrap_or(&empty);
            if let Some(until) = judge_rate_limit(status, headers, now) {
                return GitHubError::RateLimited { until };
            }
            match status {
                401 => GitHubError::Unauthorized { status },
                404 => GitHubError::NotFound {
                    path: path.to_owned(),
                },
                400 | 422 => GitHubError::Invalid {
                    method,
                    path: path.to_owned(),
                    status,
                    detail: redact(&validation_detail(&source), token),
                },
                _ => GitHubError::Status {
                    method,
                    path: path.to_owned(),
                    status,
                    message: redact(&source.message, token),
                },
            }
        }
        octocrab::Error::Serde { source, .. } => GitHubError::Decode {
            path: path.to_owned(),
            message: source.to_string(),
        },
        octocrab::Error::Json { source, .. } => GitHubError::Decode {
            path: path.to_owned(),
            message: source.to_string(),
        },
        octocrab::Error::InvalidUtf8 { source, .. } => GitHubError::Decode {
            path: path.to_owned(),
            message: source.to_string(),
        },
        other => GitHubError::Unreachable(redact(&other.to_string(), token)),
    }
}

fn validation_detail(source: &octocrab::GitHubError) -> String {
    let mut detail = if source.message.is_empty() {
        "no message".to_owned()
    } else {
        source.message.clone()
    };
    let errors: Vec<String> = source
        .errors
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|error| match error {
            Value::String(text) if !text.is_empty() => Some(text.clone()),
            Value::String(_) => None,
            error => {
                let field = [error["resource"].as_str(), error["field"].as_str()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(".");
                let said = error["message"]
                    .as_str()
                    .or_else(|| error["code"].as_str())
                    .unwrap_or_default();
                match (field.is_empty(), said.is_empty()) {
                    (false, false) => Some(format!("{field}: {said}")),
                    (false, true) => Some(field),
                    (true, false) => Some(said.to_owned()),
                    (true, true) => None,
                }
            }
        })
        .collect();
    if !errors.is_empty() {
        detail.push_str(&format!(" ({})", errors.join("; ")));
    }
    if let Some(url) = &source.documentation_url {
        detail.push_str(&format!("; see {url}"));
    }
    detail
}

/// Classifies an HTTP status + headers via [`instrukt_outside::http::judge_response`]
/// with GitHub's rate-limit rule (`429`, or `403` with `x-ratelimit-remaining: 0`
/// or `retry-after`). Returns `Some(until)` when the response is throttled.
pub(super) fn judge_rate_limit(
    status: u16,
    headers: &http::HeaderMap,
    received: Timestamp,
) -> Option<Option<Timestamp>> {
    let rule = |answer: &instrukt_outside::http::HttpAnswer<'_>| match *answer {
        instrukt_outside::http::HttpAnswer::Response {
            status,
            headers,
            received_at,
            ..
        } if Limits::read(headers).refusal_throttles(status) => {
            Some(instrukt_outside::Unavailable {
                reason: instrukt_outside::Reason::RateLimited,
                extent: instrukt_outside::Extent::Key,
                named: Limits::read(headers)
                    .until(received_at)
                    .map(instrukt_outside::Named::At),
            })
        }
        _ => None,
    };
    let settings = instrukt_outside::Settings {
        rules: &[&rule],
        ..Default::default()
    };
    match instrukt_outside::http::judge_response(status, headers, received, None, &settings) {
        instrukt_outside::http::Judgment::Unavailable(instrukt_outside::Unavailable {
            reason: instrukt_outside::Reason::RateLimited,
            named,
            ..
        }) => Some(match named {
            Some(instrukt_outside::Named::At(at)) => Some(at),
            _ => None,
        }),
        _ => None,
    }
}

/// GitHub's rate-limit header view used by GraphQL error classification and
/// sibling work modules; delegates `Retry-After` parsing to
/// [`instrukt_outside::http::retry_after`].
#[derive(Debug, Clone, Default)]
pub(super) struct Limits(http::HeaderMap);

impl Limits {
    pub(super) fn read(headers: &http::HeaderMap) -> Self {
        Self(headers.clone())
    }

    fn remaining_zero(&self) -> bool {
        self.0
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            == Some("0")
    }

    fn signalled(&self) -> bool {
        self.remaining_zero() || self.0.contains_key(http::header::RETRY_AFTER)
    }

    pub(super) fn refusal_throttles(&self, status: u16) -> bool {
        status == 429 || (status == 403 && self.signalled())
    }

    pub(super) fn until(&self, received: Timestamp) -> Option<Timestamp> {
        self.deadline(received, None)
    }

    fn deadline(&self, received: Timestamp, unnamed_delay: Option<i64>) -> Option<Timestamp> {
        let whole = received.as_second() + i64::from(received.subsec_nanosecond() > 0);
        let add = |secs: i64| {
            whole
                .checked_add(secs)
                .and_then(|s| Timestamp::from_second(s).ok())
        };
        let delay = match instrukt_outside::http::retry_after(&self.0, received) {
            Some(instrukt_outside::Named::At(at)) => Some(at),
            Some(instrukt_outside::Named::After { delay, .. }) => {
                i64::try_from(delay.as_secs()).ok().and_then(add)
            }
            None => unnamed_delay.and_then(add),
        };
        let reset = self
            .0
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
            .and_then(|s| Timestamp::from_second(s).ok());
        if self.remaining_zero() {
            delay.max(reset)
        } else {
            delay.or(reset)
        }
    }
}

/// Classifies a GraphQL answer with non-empty `errors`: secondary rate limit,
/// quota/typed `RATE_LIMITED`, all-`FORBIDDEN` refusal, or ordinary status.
pub(super) fn graphql_failure(
    errors: &[Value],
    status: u16,
    limits: &Limits,
    now: Timestamp,
    credential: &str,
) -> GitHubError {
    let typed = |entry: &Value, kind: &str| entry["type"].as_str() == Some(kind);
    let is_secondary = |entry: &Value| {
        entry["message"]
            .as_str()
            .is_some_and(|m| m.to_ascii_lowercase().contains("secondary rate"))
    };
    if errors.iter().any(is_secondary) {
        return GitHubError::RateLimited {
            until: limits.deadline(now, Some(SECONDARY_WAIT_SECONDS)),
        };
    }
    if limits.signalled() || errors.iter().any(|e| typed(e, "RATE_LIMITED")) {
        return GitHubError::RateLimited {
            until: limits.until(now),
        };
    }
    let message = redact(
        &errors
            .iter()
            .filter_map(|e| e["message"].as_str())
            .collect::<Vec<_>>()
            .join("; "),
        credential,
    );
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

fn redact(text: &str, token: &str) -> String {
    if token.is_empty() {
        text.to_owned()
    } else {
        text.replace(token, "<credential>")
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

    const NOW: i64 = 1_767_225_000;
    const RESET: i64 = 1_767_225_600;

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(second).unwrap()
    }

    fn limits(remaining: Option<&str>, reset: Option<i64>) -> Limits {
        let mut headers = http::HeaderMap::new();
        if let Some(rem) = remaining {
            headers.insert("x-ratelimit-remaining", rem.parse().unwrap());
        }
        if let Some(reset) = reset {
            headers.insert("x-ratelimit-reset", reset.to_string().parse().unwrap());
        }
        Limits::read(&headers)
    }

    fn failure(errors: Value, remaining: Option<&str>) -> GitHubError {
        let errors = errors.as_array().cloned().unwrap_or_default();
        graphql_failure(
            &errors,
            200,
            &limits(remaining, Some(RESET)),
            at(NOW),
            "secret",
        )
    }

    #[test]
    fn an_ok_secondary_limit_naming_no_delay_waits_a_minute_not_the_primary_reset() {
        let secondary = json!([{"message": "You have exceeded a secondary rate limit."}]);
        let error = failure(secondary, Some("4999"));
        assert_eq!(
            error,
            GitHubError::RateLimited {
                until: Some(at(NOW + 60))
            }
        );
        assert!(error.is_reach());
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
                until: Some(at(RESET))
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
    fn pagination_link_rejects_plaintext_downgrade_and_cross_origin_urls() {
        let base: http::Uri = "https://api.github.com".parse().unwrap();
        assert!(link_admitted(
            &base,
            &"https://api.github.com/repos/o/r/issues?page=2"
                .parse()
                .unwrap()
        ));
        assert!(link_admitted(
            &base,
            &"/repos/o/r/issues?page=2".parse().unwrap()
        ));
        assert!(!link_admitted(
            &base,
            &"http://api.github.com/repos/o/r/issues?page=2"
                .parse()
                .unwrap()
        ));
        assert!(!link_admitted(
            &base,
            &"https://evil.example/repos/o/r/issues?page=2"
                .parse()
                .unwrap()
        ));
        assert!(!link_admitted(
            &base,
            &"//api.github.com/repos/o/r/issues?page=2".parse().unwrap()
        ));
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
