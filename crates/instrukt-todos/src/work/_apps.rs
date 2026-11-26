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
    time::{Duration, Instant},
};
fn setup(cause: &str) -> AppError {
    AppError::Setup(cause.into())
}

/// A caller's `now` carried forward on the monotonic clock, so each GitHub
/// answer is dated when it is received, not when the work that awaited it
/// began. Its wall time and monotonic origin are taken together, once, where
/// the caller samples `now`; every client and issuance of that pass carries
/// this one value, so waiting on the capability lock is never left out.
#[derive(Clone, Copy)]
pub(crate) struct Clock {
    start: Timestamp,
    started: Instant,
}
impl Clock {
    /// Anchors `start`, just sampled, to this instant.
    pub(crate) fn starting(start: Timestamp) -> Self {
        Self {
            start,
            started: Instant::now(),
        }
    }
    /// The present moment, exact to the nanosecond: a stored deadline has
    /// arrived only once this reaches it. Rounding a relative wait into a
    /// deadline is [`super::_github::Limits`]'s, never the clock's.
    fn now(&self) -> Timestamp {
        self.checked_now().unwrap_or(Timestamp::MAX)
    }
    /// [`Clock::now`], or `None` past the representable range.
    pub(super) fn checked_now(&self) -> Option<Timestamp> {
        carried(self.start, self.started.elapsed())
    }
}

/// `start` carried forward by `elapsed` on the monotonic clock, exactly: no
/// fraction of a second is dropped or added. `None` past the representable
/// range.
pub(super) fn carried(start: Timestamp, elapsed: Duration) -> Option<Timestamp> {
    jiff::SignedDuration::try_from(elapsed)
        .ok()
        .and_then(|elapsed| start.checked_add(elapsed).ok())
}

/// Each App's issuance deferral: the instant GitHub permits that App's next
/// request after throttling it. It is shared by every repository and
/// operation the App serves.
#[derive(Default)]
struct Deferrals(BTreeMap<u64, Timestamp>);
impl Deferrals {
    /// The deferral of `app` still in force at the exact moment `now`: one
    /// whose instant has not yet arrived.
    fn until(&self, app: u64, now: Timestamp) -> Option<Timestamp> {
        self.0.get(&app).copied().filter(|until| *until > now)
    }
    /// Remembers the retry boundary of a throttled issuance of `app`.
    fn record(&mut self, app: u64, error: &AppError) {
        if let AppError::RateLimited { until: Some(until) } = error {
            self.0.insert(app, *until);
        }
    }
}
#[derive(Clone, Copy)]
pub(super) enum Method {
    Get,
    Post,
    Patch,
    Put,
}

/// One invocation's work-owned GitHub capabilities. The CLI shares this object
/// between work, repository and runner; no credential or key is persisted.
pub struct GitHubAccess {
    pub(super) home: Home,
    state: Mutex<Option<Apps>>,
}
impl GitHubAccess {
    pub fn new(home: Home) -> Self {
        Self {
            home,
            state: Mutex::new(None),
        }
    }
    pub(super) fn with_apps<T>(
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
            apps.credentials.retain(|(_, _, operation), _| {
                matches!(operation, Operation::Observe | Operation::ReviewObserve)
            });
        }
        Ok(())
    }
    pub(crate) fn issue(
        &self,
        repository: &str,
        role: Option<&str>,
        operation: Operation,
        clock: Clock,
    ) -> Result<Arc<Credential>, AppError> {
        self.with_apps(|apps| apps.issue(repository, role, operation, clock))
    }
    /// The configured App bots trusted as pull-request authors and the bot
    /// of the identity the `commit-reviewer` role selects, from the
    /// machine's recorded configuration without a request.
    pub(crate) fn review_identities(
        &self,
    ) -> Result<instrukt_runner::interfaces::ReviewIdentities, AppError> {
        let config = Configuration::load_global(&self.home)
            .map_err(|_| setup("cannot read machine-global configuration"))?;
        let identity = IdentitySettings::read(&config, &self.home)?;
        let reviewer = identity
            .selected(Some(instrukt_core::config::REVIEW_ROLE))
            .bot
            .clone();
        Ok(instrukt_runner::interfaces::ReviewIdentities {
            trusted_bots: identity
                .apps
                .values()
                .map(|app| app.bot.clone())
                .filter(|bot| bot != &reviewer)
                .collect(),
            reviewer,
        })
    }
    /// A client of `repository` under the publication credential of the
    /// identity the `commit-reviewer` role selects. `clock` is read here, as
    /// issuance begins, never earlier in the caller's pass.
    pub(crate) fn review_client(
        &self,
        repository: &str,
        clock: &dyn Fn() -> Timestamp,
    ) -> Result<super::_github::GitHub, AppError> {
        let clock = Clock::starting(clock());
        self.with_apps(|apps| {
            let credential = apps.issue(
                repository,
                Some(instrukt_core::config::REVIEW_ROLE),
                Operation::ReviewPublish,
                clock,
            )?;
            Ok(super::_github::GitHub::new(
                &apps.api,
                repository,
                credential.token().to_owned(),
            ))
        })
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
        let clock = Clock::starting(now);
        self.with_apps(|apps| {
            let id = apps.selected(None).app_id;
            apps.credentials.remove(&(repository.into(), id, operation));
            apps.installations.remove(&(repository.into(), id));
            apps.issue(repository, None, operation, clock)
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
        let clock = Clock::starting(now);
        self.with_apps(|apps| {
            let record = apps.issue(repository, None, Operation::Record, clock)?;
            apps.verified_bots(repository, clock)?;
            let gh = super::_github::GitHub::new(&apps.api, repository, record.token().into());
            let inspect = || -> Result<(), super::GitHubError> {
                let repository = gh.read("")?;
                if repository["has_issues"] == false {
                    return Err(setup("repository issues are disabled").into());
                }
                let issues = gh.list("/issues?state=all")?;
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
            let account = apps.issue(repository, None, Operation::Board, clock)?;
            let gh = super::_github::GitHub::new(&apps.api, repository, account.token().into());
            super::_board::inspect(&gh, board).map_err(readiness_error)
        })
    }
}

fn readiness_error(error: super::GitHubError) -> AppError {
    match error {
        super::GitHubError::App(error) => error,
        super::GitHubError::OperationApp(error) => error.into(),
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
        Operation::ReviewObserve => json!({"pull_requests":"read"}),
        Operation::ReviewPublish => json!({"pull_requests":"write","statuses":"write"}),
        Operation::CompanyRepository => json!({"administration":"write"}),
    }
}

/// The identity whose credential automatic review could not be issued.
struct Unready {
    /// Which identity, in plain words.
    identity: &'static str,
    /// Its App's slug, when the configuration names one.
    app: Option<String>,
    operation: Operation,
    error: AppError,
}

impl instrukt_core::interfaces::AutomaticReviewReadiness for GitHubAccess {
    fn review_readiness(
        &self,
        repository: &str,
        now: Timestamp,
    ) -> instrukt_core::interfaces::ReviewReadiness {
        use instrukt_core::interfaces::ReviewReadiness;
        let clock = Clock::starting(now);
        let answer = self.with_apps(|apps| {
            let unready = |apps: &Apps, identity, role, operation, error| Unready {
                identity,
                app: Some(apps.selected(role).slug.clone()),
                operation,
                error,
            };
            let observation = match apps.issue(repository, None, Operation::ReviewObserve, clock) {
                Ok(credential) => credential,
                Err(error) => {
                    return Ok(Err(unready(
                        apps,
                        "project",
                        None,
                        Operation::ReviewObserve,
                        error,
                    )));
                }
            };
            let role = Some(instrukt_core::config::REVIEW_ROLE);
            Ok(
                match apps.issue(repository, role, Operation::ReviewPublish, clock) {
                    Ok(_) => Ok(observation),
                    Err(error) => Err(unready(
                        apps,
                        "reviewer",
                        role,
                        Operation::ReviewPublish,
                        error,
                    )),
                },
            )
        });
        let unready = match answer {
            Ok(Ok(observation)) => return ReviewReadiness::Met { observation },
            Ok(Err(unready)) => unready,
            Err(error) => Unready {
                identity: "configured",
                app: None,
                operation: Operation::ReviewObserve,
                error,
            },
        };
        match &unready.error {
            AppError::Unreachable | AppError::RateLimited { .. } => {
                return ReviewReadiness::CannotAsk(unready.error.to_string());
            }
            AppError::Refused { status, .. } if *status >= 500 => {
                return ReviewReadiness::CannotAsk(unready.error.to_string());
            }
            _ => {}
        }
        let needed = permissions(unready.operation)
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, level)| format!("{name}: {}", level.as_str().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join(", ");
        let app = unready
            .app
            .map(|slug| format!(" App `{slug}`"))
            .unwrap_or_default();
        tracing::info!(event = "review_readiness_unmet", repository, identity = unready.identity, error = %unready.error);
        ReviewReadiness::Unmet(format!(
            "the {} identity's{app} cannot do its part for automatic review on {repository}: {}; grant its installation on {repository} {needed}",
            unready.identity, unready.error
        ))
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
    /// The App a recorded `role` selects: the `github_app` of the role
    /// configuration says now governs it (a retired split orchestrator's
    /// work selects `todo-orchestrator`'s), else the project App.
    fn selected(&self, role: Option<&str>) -> &App {
        let name = role
            .map(instrukt_core::config::current_role)
            .and_then(|r| self.roles.get(r))
            .unwrap_or(&self.project);
        &self.apps[name]
    }
}
pub(super) struct Apps {
    home: Home,
    api: String,
    identity: IdentitySettings,
    agent: ureq::Agent,
    auth_deferrals: Deferrals,
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
        let api = api_base(&config)?;
        Ok(Self {
            home: home.clone(),
            api,
            identity,
            agent: agent(),
            auth_deferrals: Default::default(),
            actors: Default::default(),
            verified_apps: Default::default(),
            installations: Default::default(),
            credentials: Default::default(),
        })
    }
    pub(super) fn selected(&self, role: Option<&str>) -> &App {
        self.identity.selected(role)
    }
    pub(super) fn issue(
        &mut self,
        repository: &str,
        role: Option<&str>,
        operation: Operation,
        clock: Clock,
    ) -> Result<Arc<Credential>, AppError> {
        let now = clock.now();
        let app = self.selected(role).clone();
        let key = (repository.to_owned(), app.app_id, operation);
        if let Some(until) = self.auth_deferrals.until(app.app_id, now) {
            return Err(AppError::RateLimited { until: Some(until) });
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
            .issue_uncached(repository, &app, operation, clock)
            .map(Arc::new);
        tracing::debug!(event="github_credential_acquired",app_id=app.app_id,app_slug=%app.slug,repository,operation=?operation,outcome=if result.is_err(){"failed"}else if renewal{"renewed"}else{"issued"},duration_ms=u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX));
        if result.is_err() {
            self.installations
                .remove(&(repository.to_owned(), app.app_id));
        }
        if let Err(error) = &result {
            self.auth_deferrals.record(app.app_id, error);
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
        clock: Clock,
    ) -> Result<Credential, AppError> {
        let now = clock.now();
        let key = instrukt_core::config::resolve_secret(&self.home, &app.private_key).map_err(|_| {
            setup(
                "selected App key unavailable in owner-only secrets.env; use instrukt secrets set",
            )
        })?;
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(key.as_bytes())
            .map_err(|_| setup("selected App key is not usable RSA PEM"))?;
        let jwt = jsonwebtoken::encode(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),&json!({"iat":now.as_second()-60,"exp":now.as_second()+540,"iss":app.app_id.to_string()}),&key).map_err(|_|setup("cannot sign selected App JWT"))?;
        if !self.verified_apps.contains(&app.app_id) {
            let identity = self.request(Method::Get, "/app", &jwt, None, clock)?;
            if identity["id"].as_u64() != Some(app.app_id)
                || identity["slug"].as_str() != Some(&app.slug)
            {
                return Err(setup(
                    "selected key's App identity does not match configuration",
                ));
            }
            self.verified_apps.insert(app.app_id);
        }
        // A company repository's credential is the installation's on the
        // organisation named in `repository`. It is requested without a
        // repository restriction, so it reaches every repository available
        // to that installation.
        let organisation = op == Operation::CompanyRepository;
        let installation_endpoint = if organisation {
            format!("/orgs/{repository}/installation")
        } else {
            format!("/repos/{repository}/installation")
        };
        let installation = match self.installations.get(&(repository.into(), app.app_id)) {
            Some(installation) => installation.clone(),
            None => self.request(Method::Get, &installation_endpoint, &jwt, None, clock)?,
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
        let unscoped = op == Operation::Board || organisation;
        let token_request = if unscoped {
            json!({"permissions": permissions})
        } else {
            let (_, repo) = repository
                .split_once('/')
                .ok_or_else(|| setup("invalid repository"))?;
            json!({"repositories": [repo], "permissions": permissions})
        };
        let response = self.request(
            Method::Post,
            &format!("/app/installations/{id}/access_tokens"),
            &jwt,
            Some(&token_request),
            clock,
        )?;
        check_permissions(&response["permissions"], &permissions)?;
        let expires: Timestamp = response["expires_at"]
            .as_str()
            .and_then(|s| s.parse::<Timestamp>().ok())
            .filter(|t| t.as_second() > clock.now().as_second() + 30)
            .ok_or_else(|| setup("issued credential has no usable expiry"))?;
        let token = response["token"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| setup("issuance returned no installation credential"))?;
        if !unscoped {
            let repos = self.request(
                Method::Get,
                "/installation/repositories?per_page=100",
                token,
                None,
                clock,
            )?;
            if !repos["repositories"].as_array().is_some_and(|rs| {
                rs.iter()
                    .any(|r| r["full_name"].as_str() == Some(repository))
            }) {
                return Err(setup("issued credential cannot reach repository"));
            }
        }
        let viewer = self.request(
            Method::Post,
            "/graphql",
            token,
            Some(&json!({"query":"query { viewer { id login databaseId } }"})),
            clock,
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
    fn verified_bots(&mut self, repository: &str, clock: Clock) -> Result<Vec<Bot>, AppError> {
        let credential = self.issue(repository, None, Operation::Record, clock)?;
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
                clock,
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
    /// One App API request; a throttled answer's retry boundary counts from
    /// when `clock` says the answer was received.
    fn request(
        &self,
        method: Method,
        endpoint: &str,
        bearer: &str,
        body: Option<&Value>,
        clock: Clock,
    ) -> Result<Value, AppError> {
        let Answer {
            status,
            body: answer,
            limits,
            received,
        } = self.send(method, endpoint, bearer, body, clock)?;
        if !(200..300).contains(&status) {
            return Err(AppError::Refused {
                operation: endpoint.split('?').next().unwrap_or(endpoint).into(),
                status,
            });
        }
        // GraphQL reports throttling, primary or secondary, in a successful
        // answer's `errors`; its retry boundary is the headers' to name, or
        // GitHub's minute for a secondary limit that names no delay.
        if let Some(errors) = answer["errors"].as_array().filter(|e| !e.is_empty())
            && let super::GitHubError::RateLimited { until } =
                super::_github::graphql_failure(errors, status, &limits, received, bearer)
        {
            return Err(AppError::RateLimited { until });
        }
        Ok(answer)
    }
    /// One App API request answered with any status but a throttling one:
    /// the status and the body as JSON (`null` when empty, or when a refusal
    /// carries no JSON). Unreachable and throttled answers are errors.
    pub(super) fn send(
        &self,
        method: Method,
        endpoint: &str,
        bearer: &str,
        body: Option<&Value>,
        clock: Clock,
    ) -> Result<Answer, AppError> {
        if bearer.is_empty() {
            return Err(setup("authenticated App capability is unavailable"));
        }
        let url = match (endpoint, self.api.strip_suffix("/api/v3")) {
            ("/graphql", Some(host)) => format!("{host}/api/graphql"),
            _ => format!("{}{endpoint}", self.api),
        };
        let payload = body.map(Value::to_string).unwrap_or_default();
        let mut response = match method {
            Method::Post => headers(self.agent.post(&url), bearer)
                .header("Content-Type", "application/json")
                .send(payload),
            Method::Patch => headers(self.agent.patch(&url), bearer)
                .header("Content-Type", "application/json")
                .send(payload),
            Method::Put => headers(self.agent.put(&url), bearer)
                .header("Content-Type", "application/json")
                .send(payload),
            Method::Get => headers(self.agent.get(&url), bearer).call(),
        }
        .map_err(|_| AppError::Unreachable)?;
        let received = clock.now();
        let status = response.status().as_u16();
        let limits = super::_github::Limits::read(response.headers());
        if limits.refusal_throttles(status) {
            return Err(AppError::RateLimited {
                until: limits.until(received),
            });
        }
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|_| AppError::Unreachable)?;
        let body = if text.trim().is_empty() {
            Value::Null
        } else {
            match serde_json::from_str(&text) {
                Ok(body) => body,
                Err(_) if !(200..300).contains(&status) => Value::Null,
                Err(_) => return Err(setup("unreadable App API response")),
            }
        };
        Ok(Answer {
            status,
            body,
            limits,
            received,
        })
    }
}

/// One App API answer [`Apps::send`] received.
pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) body: Value,
    limits: super::_github::Limits,
    received: Timestamp,
}
/// The GitHub API address `config` declares, without a trailing `/`: HTTPS,
/// or HTTP on loopback only.
pub(super) fn api_base(config: &Configuration) -> Result<String, AppError> {
    let api = config
        .github_api_url()
        .map_err(|_| setup("cannot read github.api_url"))?;
    let url = url::Url::parse(api).map_err(|_| setup("invalid github.api_url"))?;
    if url.scheme() != "https"
        && !(url.scheme() == "http"
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    {
        return Err(setup(
            "github.api_url requires HTTPS (HTTP only on loopback)",
        ));
    }
    Ok(api.trim_end_matches('/').into())
}

pub(super) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .timeout_connect(Some(Duration::from_secs(10)))
        .max_redirects(0)
        .user_agent("instrukt")
        .build()
        .into()
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

#[cfg(test)]
mod tests {
    use super::*;
    use instrukt_core::github::Bot;

    fn app(id: u64, slug: &str) -> App {
        App {
            app_id: id,
            slug: slug.into(),
            bot: Bot {
                id: id + 100,
                node_id: format!("BOT_{id}"),
                login: format!("{slug}[bot]"),
            },
            private_key: format!("${{APP_{id}}}"),
        }
    }

    fn settings(roles: &[(&str, &str)]) -> IdentitySettings {
        IdentitySettings {
            apps: [
                ("project".into(), app(1, "project")),
                ("author".into(), app(2, "author")),
            ]
            .into(),
            project: "project".into(),
            roles: roles
                .iter()
                .map(|(role, app)| ((*role).into(), (*app).into()))
                .collect(),
        }
    }

    /// A whole second the moments below count from; GitHub's primary reset
    /// in them, 2025-12-31T23:50:10Z, is ten seconds later.
    const START: i64 = 1_767_225_000;
    const RESET: i64 = START + 10;

    /// `second` plus `millis` milliseconds, which may be negative.
    fn moment(second: i64, millis: i64) -> Timestamp {
        Timestamp::from_second(second)
            .unwrap()
            .checked_add(jiff::SignedDuration::from_millis(millis))
            .unwrap()
    }

    /// The throttling signals of an answer with these headers.
    fn limits(headers: &[(&'static str, &str)]) -> crate::work::_github::Limits {
        let mut map = ureq::http::HeaderMap::new();
        for (name, value) in headers {
            map.insert(*name, value.parse().unwrap());
        }
        crate::work::_github::Limits::read(&map)
    }

    /// App 1's deferral as issuance keeps it after a throttled answer with
    /// `limits` received at `received`.
    fn deferred(limits: &crate::work::_github::Limits, received: Timestamp) -> Deferrals {
        let mut deferrals = Deferrals::default();
        deferrals.record(
            1,
            &AppError::RateLimited {
                until: limits.until(received),
            },
        );
        deferrals
    }

    #[test]
    fn a_clock_carries_a_fractional_start_forward_by_exactly_the_time_elapsed() {
        let carried = carried(moment(START, 800), Duration::from_millis(800));
        assert_eq!(carried, Some(moment(START, 1_600)));
    }

    #[test]
    fn a_cached_reset_defers_issuance_until_its_exact_instant() {
        let reset = RESET.to_string();
        let limits = limits(&[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", &reset),
        ]);
        let deferrals = deferred(&limits, moment(START, 0));
        assert_eq!(
            deferrals.until(1, moment(RESET, -200)),
            Some(moment(RESET, 0))
        );
        assert_eq!(deferrals.until(1, moment(RESET, 0)), None);
        assert_eq!(deferrals.until(1, moment(RESET, 200)), None);
    }

    #[test]
    fn a_cached_named_delay_defers_issuance_its_full_length_after_a_fractional_receipt() {
        // Answered 0.8 s after a clock that began 0.8 s into its second.
        let received = carried(moment(START, 800), Duration::from_millis(800)).unwrap();
        let deferrals = deferred(&limits(&[("retry-after", "2")]), received);
        // Two seconds from a receipt at START + 1.6 s end at START + 3.6 s;
        // the deadline is the whole second after.
        assert_eq!(
            deferrals.until(1, moment(START + 3, 800)),
            Some(moment(START + 4, 0))
        );
        assert_eq!(deferrals.until(1, moment(START + 4, 0)), None);
        assert_eq!(deferrals.until(1, moment(START + 4, 200)), None);
    }

    #[test]
    fn a_retired_orchestrator_binding_selects_the_unified_orchestrator_app() {
        let identity = settings(&[("todo-orchestrator", "author")]);
        assert_eq!(identity.selected(Some("prepare-orchestrator")).app_id, 2);
        assert_eq!(identity.selected(Some("work-orchestrator")).app_id, 2);
    }

    #[test]
    fn an_unmapped_unified_orchestrator_leaves_retired_bindings_on_the_project_app() {
        let identity = settings(&[]);
        assert_eq!(identity.selected(Some("prepare-orchestrator")).app_id, 1);
        assert_eq!(identity.selected(Some("work-orchestrator")).app_id, 1);
        assert_eq!(identity.selected(None).app_id, 1);
    }
}
