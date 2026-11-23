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
/// began.
#[derive(Clone, Copy)]
struct Clock {
    start: Timestamp,
    started: Instant,
}
impl Clock {
    fn starting(start: Timestamp) -> Self {
        Self {
            start,
            started: Instant::now(),
        }
    }
    /// The present moment, exact to the nanosecond: a stored deadline has
    /// arrived only once this reaches it. Rounding a relative wait into a
    /// deadline is [`super::_github::Limits`]'s, never the clock's.
    fn now(&self) -> Timestamp {
        carried(self.start, self.started.elapsed()).unwrap_or(Timestamp::MAX)
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
        let clock = Clock::starting(now);
        self.with_apps(|apps| apps.issue(repository, role, operation, clock))
    }
    pub(crate) fn verified_bots(
        &self,
        repository: &str,
        now: Timestamp,
    ) -> Result<Vec<Bot>, AppError> {
        let clock = Clock::starting(now);
        self.with_apps(|apps| apps.verified_bots(repository, clock))
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
struct Apps {
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
        Ok(Self {
            home: home.clone(),
            api: api.trim_end_matches('/').into(),
            identity,
            agent: agent(),
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
        let installation = match self.installations.get(&(repository.into(), app.app_id)) {
            Some(installation) => installation.clone(),
            None => self.request(
                Method::Get,
                &format!("/repos/{repository}/installation"),
                &jwt,
                None,
                clock,
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
        let token_request = if op == Operation::Board {
            json!({"permissions": permissions})
        } else {
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
        if op != Operation::Board {
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
        let received = clock.now();
        let status = response.status().as_u16();
        let limits = super::_github::Limits::read(response.headers());
        if limits.refusal_throttles(status) {
            return Err(AppError::RateLimited {
                until: limits.until(received),
            });
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
        let answer: Value =
            serde_json::from_str(&text).map_err(|_| setup("unreadable App API response"))?;
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
}
fn agent() -> ureq::Agent {
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

    /// The moment an issuance begins in the tests below; the primary reset
    /// their answers carry is ten minutes later.
    const START: i64 = 1_767_225_000;
    const RESET: i64 = START + 600;

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(second).unwrap()
    }

    /// The viewer query of an issuance, answered on loopback by HTTP 200
    /// with GitHub's secondary rate limit in `errors`, requests remaining,
    /// a later primary reset and `retry_after` when given.
    fn viewer_throttled(retry_after: Option<u64>, clock: Clock) -> Result<Value, AppError> {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new();
                request.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            request.read_exact(&mut vec![0; length]).unwrap();
            let body = r#"{"errors":[{"message":"You have exceeded a secondary rate limit. Please wait a few minutes before you try again."}]}"#;
            let delay = retry_after.map_or(String::new(), |s| format!("Retry-After: {s}\r\n"));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-RateLimit-Remaining: 4999\r\nX-RateLimit-Reset: {RESET}\r\n{delay}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let home = tempfile::tempdir().unwrap();
        let apps = Apps {
            home: Home::at_user_home(home.path().into()),
            api,
            identity: settings(&[]),
            agent: agent(),
            auth_deferrals: Default::default(),
            actors: Default::default(),
            verified_apps: Default::default(),
            installations: Default::default(),
            credentials: Default::default(),
        };
        let query = json!({"query": "query { viewer { id login databaseId } }"});
        let answer = apps.request(Method::Post, "/graphql", "token", Some(&query), clock);
        server.join().unwrap();
        answer
    }

    #[test]
    fn a_throttled_answer_counts_its_delay_from_receipt_not_from_when_issuance_began() {
        // Earlier issuance requests took three seconds before this answer.
        let clock = Clock {
            start: at(START),
            started: Instant::now().checked_sub(Duration::from_secs(3)).unwrap(),
        };
        let Err(AppError::RateLimited { until: Some(until) }) = viewer_throttled(Some(2), clock)
        else {
            panic!("a secondary limit naming a delay is a rate limit");
        };
        assert!(
            (at(START + 3 + 2)..at(START + 3 + 2 + 30)).contains(&until),
            "{until} must permit the full delay after receipt"
        );
    }

    #[test]
    fn a_secondary_limit_naming_no_delay_defers_a_minute_and_is_no_actor_mismatch() {
        let clock = Clock::starting(at(START));
        let Err(AppError::RateLimited { until: Some(until) }) = viewer_throttled(None, clock)
        else {
            panic!("a secondary limit is a rate limit, never a viewer answer");
        };
        // GitHub's minute, not the primary reset that quota still allows.
        assert!(
            (at(START + 60)..at(START + 60 + 30)).contains(&until),
            "{until} must be a minute after receipt"
        );
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
