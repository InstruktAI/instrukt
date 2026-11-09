//! Work-owned verified actors and bounded installation credentials.
use instrukt_core::{
    config::{Agents, Configuration},
    github::{App, AppError, Bot, Credential, Operation},
    home::Home,
};
use jiff::Timestamp;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};
fn setup(cause: &str) -> AppError {
    AppError::Setup(cause.into())
}
#[derive(Clone, Copy)]
enum Method {
    Get,
    Post,
}

/// One invocation's work-owned GitHub capabilities. The CLI shares this object
/// between work, repository and runner; no credential or key is persisted.
pub struct GitHubAccess {
    home: Home,
    state: Mutex<Option<Apps>>,
}
impl GitHubAccess {
    pub fn new(home: Home) -> Self {
        Self {
            home,
            state: Mutex::new(None),
        }
    }
    fn with_apps<T>(
        &self,
        action: impl FnOnce(&mut Apps) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| setup("GitHub capability lock failed"))?;
        let current = Apps::load(&self.home)?;
        let unchanged = state
            .as_ref()
            .is_some_and(|apps| apps.api == current.api && apps.identity == current.identity);
        if !unchanged {
            *state = Some(current);
        }
        action(
            state
                .as_mut()
                .ok_or_else(|| setup("GitHub configuration unavailable"))?,
        )
    }
    pub(crate) fn validate(&self) -> Result<(), AppError> {
        self.with_apps(|_| Ok(()))
    }
    pub(crate) fn begin_pass(&self) -> Result<(), AppError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| setup("GitHub capability lock failed"))?;
        if let Some(apps) = state.as_mut() {
            apps.installations.clear();
            apps.credentials
                .retain(|(_, _, operation), _| *operation == Operation::Observe);
        }
        Ok(())
    }
    pub(crate) fn issue(
        &self,
        repository: &str,
        role: Option<&str>,
        operation: Operation,
        now: Timestamp,
    ) -> Result<Arc<Credential>, AppError> {
        self.with_apps(|apps| apps.issue(repository, role, operation, now))
    }
    pub(crate) fn verified_bots(
        &self,
        repository: &str,
        now: Timestamp,
    ) -> Result<Vec<Bot>, AppError> {
        self.with_apps(|apps| apps.verified_bots(repository, now))
    }
}
impl instrukt_core::interfaces::GitHubActors for GitHubAccess {
    fn actor(&self, role: &str) -> Result<Bot, AppError> {
        // Setup records the verified stable bot tuple in machine configuration.
        // Local attribution consumes those facts; it grants no remote access.
        let config = Configuration::load_global(&self.home)
            .map_err(|_| setup("cannot read machine-global configuration"))?;
        Ok(IdentitySettings::read(&config, &self.home)?
            .selected(Some(role))
            .bot
            .clone())
    }
}
impl instrukt_runner::interfaces::GitHubCredentials for GitHubAccess {
    fn credential(
        &self,
        repository: &str,
        operation: Operation,
        now: Timestamp,
    ) -> Result<Arc<Credential>, AppError> {
        self.with_apps(|apps| {
            let id = apps.selected(None).app_id;
            apps.credentials.remove(&(repository.into(), id, operation));
            apps.installations.remove(&(repository.into(), id));
            apps.issue(repository, None, operation, now)
        })
    }
}
impl instrukt_runner::interfaces::GitHubReadiness for GitHubAccess {
    fn reflection_ready(
        &self,
        repository: &str,
        board: Option<&instrukt_core::config::BoardRef>,
        now: Timestamp,
    ) -> Result<(), AppError> {
        self.with_apps(|apps| {
            let record = apps.issue(repository, None, Operation::Record, now)?;
            apps.verified_bots(repository, now)?;
            let gh = super::_github::GitHub::new(&apps.api, repository, record.token().into());
            let inspect = || -> Result<(), super::GitHubError> {
                let repository = gh.read("")?;
                if repository["has_issues"] == false {
                    return Err(setup("repository issues are disabled").into());
                }
                let issues = gh.read("/issues?state=all&per_page=100")?;
                let issues = issues
                    .as_array()
                    .ok_or_else(|| setup("unreadable issue listing"))?;
                if let Some(number) = issues
                    .iter()
                    .find(|issue| issue.get("pull_request").is_none())
                    .and_then(|issue| issue["number"].as_u64())
                {
                    gh.list(&format!("/issues/{number}/artifacts"))?;
                }
                Ok(())
            };
            inspect().map_err(readiness_error)?;
            let account = apps.issue(repository, None, Operation::Board, now)?;
            let gh = super::_github::GitHub::new(&apps.api, repository, account.token().into());
            super::_board::inspect(&gh, board).map_err(readiness_error)
        })
    }
}

fn readiness_error(error: super::GitHubError) -> AppError {
    match error {
        super::GitHubError::App(error) => error,
        super::GitHubError::RateLimited { until } => AppError::RateLimited { until },
        super::GitHubError::Unreachable(_) => AppError::Unreachable,
        error => setup(&error.to_string()),
    }
}

fn permissions(operation: Operation) -> Value {
    match operation {
        Operation::Record => json!({"issues":"write","contents":"read","pull_requests":"read"}),
        Operation::Author => json!({"contents":"write","pull_requests":"write"}),
        Operation::Review => json!({"contents":"read","pull_requests":"write"}),
        Operation::Board => json!({"organization_projects":"write"}),
        Operation::Observe => json!({"checks":"read","statuses":"read","contents":"read"}),
    }
}
#[derive(PartialEq, Eq)]
struct IdentitySettings {
    apps: BTreeMap<String, App>,
    project: String,
    roles: BTreeMap<String, String>,
}
impl IdentitySettings {
    fn read(config: &Configuration, home: &Home) -> Result<Self, AppError> {
        let apps = config
            .github_apps()
            .map_err(|_| setup("cannot read github.apps"))?;
        let project = config
            .github_project_app()
            .map_err(|_| setup("cannot read github.project_app"))?
            .ok_or_else(|| setup("github.project_app is missing"))?
            .to_owned();
        let agents = Agents::load(home).map_err(|_| setup("cannot read agents.yaml roles"))?;
        let roles: BTreeMap<_, _> = agents
            .roles
            .into_iter()
            .filter_map(|(r, s)| s.github_app.map(|a| (r, a)))
            .collect();
        if !apps.contains_key(&project) || roles.values().any(|r| !apps.contains_key(r)) {
            return Err(setup(
                "github.project_app or roles.<role>.github_app refers to an unknown profile",
            ));
        }
        Ok(Self {
            apps,
            project,
            roles,
        })
    }
    fn selected(&self, role: Option<&str>) -> &App {
        let name = role
            .and_then(|r| self.roles.get(r))
            .unwrap_or(&self.project);
        &self.apps[name]
    }
}
struct Apps {
    home: Home,
    api: String,
    identity: IdentitySettings,
    agent: ureq::Agent,
    auth_deferrals: BTreeMap<u64, Timestamp>,
    actors: BTreeMap<String, Bot>,
    verified_apps: BTreeSet<u64>,
    installations: BTreeMap<(String, u64), Value>,
    credentials: BTreeMap<(String, u64, Operation), Arc<Credential>>,
}
impl Apps {
    pub fn load(home: &Home) -> Result<Self, AppError> {
        let config = Configuration::load_global(home)
            .map_err(|_| setup("cannot read machine-global configuration"))?;
        let identity = IdentitySettings::read(&config, home)?;
        let api = config
            .github_api_url()
            .map_err(|_| setup("cannot read github.api_url"))?
            .to_owned();
        let url = url::Url::parse(&api).map_err(|_| setup("invalid github.api_url"))?;
        if url.scheme() != "https"
            && !(url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
        {
            return Err(setup(
                "github.api_url requires HTTPS (HTTP only on loopback)",
            ));
        }
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .timeout_connect(Some(Duration::from_secs(10)))
            .max_redirects(0)
            .user_agent("instrukt")
            .build()
            .into();
        Ok(Self {
            home: home.clone(),
            api: api.trim_end_matches('/').into(),
            identity,
            agent,
            auth_deferrals: Default::default(),
            actors: Default::default(),
            verified_apps: Default::default(),
            installations: Default::default(),
            credentials: Default::default(),
        })
    }
    fn selected(&self, role: Option<&str>) -> &App {
        self.identity.selected(role)
    }
    fn issue(
        &mut self,
        repository: &str,
        role: Option<&str>,
        operation: Operation,
        now: Timestamp,
    ) -> Result<Arc<Credential>, AppError> {
        let app = self.selected(role).clone();
        let key = (repository.to_owned(), app.app_id, operation);
        if let Some(until) = self
            .auth_deferrals
            .get(&app.app_id)
            .filter(|until| **until > now)
        {
            return Err(AppError::RateLimited {
                until: Some(*until),
            });
        }

        if let Some(credential) = self
            .credentials
            .get(&key)
            .filter(|c| c.expires.as_second() > now.as_second() + 30)
        {
            return Ok(credential.clone());
        }
        let renewal = self.credentials.contains_key(&key);
        let start = std::time::Instant::now();
        let result = self
            .issue_uncached(repository, &app, operation, now)
            .map(Arc::new);
        tracing::debug!(event="github_credential_acquired",app_id=app.app_id,app_slug=%app.slug,repository,operation=?operation,outcome=if result.is_err(){"failed"}else if renewal{"renewed"}else{"issued"},duration_ms=u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX));
        if result.is_err() {
            self.installations
                .remove(&(repository.to_owned(), app.app_id));
        }
        if let Err(AppError::RateLimited { until: Some(until) }) = &result {
            self.auth_deferrals.insert(app.app_id, *until);
        }
        if let Ok(credential) = &result {
            self.credentials.insert(key, credential.clone());
        }
        result
    }
    fn issue_uncached(
        &mut self,
        repository: &str,
        app: &App,
        op: Operation,
        now: Timestamp,
    ) -> Result<Credential, AppError> {
        let key = instrukt_core::config::resolve_secret(&self.home, &app.private_key).map_err(|_| {
            setup(
                "selected App key unavailable in owner-only secrets.env; use instrukt secrets set",
            )
        })?;
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(key.as_bytes())
            .map_err(|_| setup("selected App key is not usable RSA PEM"))?;
        let jwt = jsonwebtoken::encode(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),&json!({"iat":now.as_second()-60,"exp":now.as_second()+540,"iss":app.app_id.to_string()}),&key).map_err(|_|setup("cannot sign selected App JWT"))?;
        if !self.verified_apps.contains(&app.app_id) {
            let identity = self.request(Method::Get, "/app", &jwt, None, now)?;
            if identity["id"].as_u64() != Some(app.app_id)
                || identity["slug"].as_str() != Some(&app.slug)
            {
                return Err(setup(
                    "selected key's App identity does not match configuration",
                ));
            }
            self.verified_apps.insert(app.app_id);
        }
        let installation = match self.installations.get(&(repository.into(), app.app_id)) {
            Some(installation) => installation.clone(),
            None => self.request(
                Method::Get,
                &format!("/repos/{repository}/installation"),
                &jwt,
                None,
                now,
            )?,
        };
        if !installation["suspended_at"].is_null() || !installation["suspended_by"].is_null() {
            return Err(setup("repository installation is suspended"));
        }
        self.installations
            .insert((repository.into(), app.app_id), installation.clone());
        let id = installation["id"]
            .as_u64()
            .filter(|_| installation["app_id"].as_u64() == Some(app.app_id))
            .ok_or_else(|| setup("repository installation does not match selected App"))?;
        let permissions = permissions(op);
        check_permissions(&installation["permissions"], &permissions)?;
        let (_, repo) = repository
            .split_once('/')
            .ok_or_else(|| setup("invalid repository"))?;
        let response = self.request(
            Method::Post,
            &format!("/app/installations/{id}/access_tokens"),
            &jwt,
            Some(&json!({"repositories":[repo],"permissions":permissions})),
            now,
        )?;
        check_permissions(&response["permissions"], &permissions)?;
        let expires: Timestamp = response["expires_at"]
            .as_str()
            .and_then(|s| s.parse::<Timestamp>().ok())
            .filter(|t| t.as_second() > now.as_second() + 30)
            .ok_or_else(|| setup("issued credential has no usable expiry"))?;
        let token = response["token"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| setup("issuance returned no installation credential"))?;
        let repos = self.request(
            Method::Get,
            "/installation/repositories?per_page=100",
            token,
            None,
            now,
        )?;
        if !repos["repositories"].as_array().is_some_and(|rs| {
            rs.iter()
                .any(|r| r["full_name"].as_str() == Some(repository))
        }) {
            return Err(setup("issued credential cannot reach repository"));
        }
        let viewer = self.request(
            Method::Post,
            "/graphql",
            token,
            Some(&json!({"query":"query { viewer { id login databaseId } }"})),
            now,
        )?;
        let viewer = &viewer["data"]["viewer"];
        if viewer["databaseId"].as_u64() != Some(app.bot.id)
            || viewer["id"].as_str() != Some(&app.bot.node_id)
            || viewer["login"].as_str() != Some(&app.bot.login)
        {
            return Err(setup(
                "issued credential's actor does not match the selected App bot",
            ));
        }
        self.actors.insert(app.bot.login.clone(), app.bot.clone());
        Ok(Credential::new(token.into(), app.bot.clone(), expires))
    }
    fn verified_bots(&mut self, repository: &str, now: Timestamp) -> Result<Vec<Bot>, AppError> {
        let credential = self.issue(repository, None, Operation::Record, now)?;
        let apps = self.identity.apps.values().cloned().collect::<Vec<_>>();
        for app in &apps {
            if self.actors.get(&app.bot.login) == Some(&app.bot) {
                continue;
            }
            let bot = self.request(
                Method::Get,
                &format!("/users/{}", app.bot.login),
                credential.token(),
                None,
                now,
            )?;
            if !app.bot.matches(&bot)
                || bot["login"].as_str() != Some(&app.bot.login)
                || bot["type"] != "Bot"
            {
                return Err(setup(
                    "configured App/bot stable identity does not match GitHub",
                ));
            }
            self.actors.insert(app.bot.login.clone(), app.bot.clone());
        }
        Ok(apps.into_iter().map(|app| app.bot).collect())
    }
    fn request(
        &self,
        method: Method,
        endpoint: &str,
        bearer: &str,
        body: Option<&Value>,
        now: Timestamp,
    ) -> Result<Value, AppError> {
        if bearer.is_empty() {
            return Err(setup("authenticated App capability is unavailable"));
        }
        let url = match (endpoint, self.api.strip_suffix("/api/v3")) {
            ("/graphql", Some(host)) => format!("{host}/api/graphql"),
            _ => format!("{}{endpoint}", self.api),
        };
        let mut response = match method {
            Method::Post => headers(self.agent.post(&url), bearer)
                .header("Content-Type", "application/json")
                .send(body.map(Value::to_string).unwrap_or_default()),
            Method::Get => headers(self.agent.get(&url), bearer).call(),
        }
        .map_err(|_| AppError::Unreachable)?;
        let status = response.status().as_u16();
        let h = response.headers();
        if status == 429
            || (status == 403
                && (h.get("x-ratelimit-remaining").is_some_and(|h| h == "0")
                    || h.contains_key("retry-after")))
        {
            let until = h
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
                .and_then(|v| Timestamp::from_second(v).ok());
            let until = until.or_else(|| {
                h.get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<i64>().ok())
                    .filter(|seconds| *seconds >= 0)
                    .and_then(|seconds| now.as_second().checked_add(seconds))
                    .and_then(|second| Timestamp::from_second(second).ok())
            });
            return Err(AppError::RateLimited { until });
        }
        if !(200..300).contains(&status) {
            return Err(AppError::Refused {
                operation: endpoint.split('?').next().unwrap_or(endpoint).into(),
                status,
            });
        }
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|_| AppError::Unreachable)?;
        serde_json::from_str(&text).map_err(|_| setup("unreadable App API response"))
    }
}
fn check_permissions(actual: &Value, required: &Value) -> Result<(), AppError> {
    for (name, wanted) in required.as_object().into_iter().flatten() {
        if actual[name] != *wanted && !(wanted == "read" && actual[name] == "write") {
            return Err(setup(&format!(
                "installation credential lacks required {name} permission"
            )));
        }
    }
    Ok(())
}

fn headers<B>(request: ureq::RequestBuilder<B>, bearer: &str) -> ureq::RequestBuilder<B> {
    let request = request
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    request.header("Authorization", &format!("Bearer {bearer}"))
}
