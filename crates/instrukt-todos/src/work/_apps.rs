//! Work-owned verified actors and bounded installation credentials backed by
//! [`octocrab`] App authentication and [`instrukt_outside::Gate`].
use instrukt_core::{
    config::{Agents, Configuration},
    github::{App, AppError, Bot, Credential, Operation},
    home::Home,
};
use jiff::Timestamp;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

fn setup(cause: &str) -> AppError {
    AppError::Setup(cause.into())
}

/// A caller's `now` carried forward on the monotonic clock so each GitHub
/// answer is dated when received.
#[derive(Clone, Copy)]
pub(crate) struct Clock {
    start: Timestamp,
    started: Instant,
}
impl Clock {
    pub(crate) fn starting(start: Timestamp) -> Self {
        Self {
            start,
            started: Instant::now(),
        }
    }
    fn now(&self) -> Timestamp {
        self.checked_now().unwrap_or(Timestamp::MAX)
    }
    pub(super) fn checked_now(&self) -> Option<Timestamp> {
        jiff::SignedDuration::try_from(self.started.elapsed())
            .ok()
            .and_then(|elapsed| self.start.checked_add(elapsed).ok())
    }
}

#[derive(Clone, Copy)]
pub(super) enum Method {
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
        if !state
            .as_ref()
            .is_some_and(|apps| apps.api == current.api && apps.identity == current.identity)
        {
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
    pub(crate) fn gate(&self) -> Result<instrukt_outside::Gate, AppError> {
        self.with_apps(|apps| Ok(apps.gate.clone()))
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
    pub(crate) fn review_client(
        &self,
        repository: &str,
        clock: &dyn Fn() -> Timestamp,
    ) -> Result<super::_github::GitHub, AppError> {
        self.role_client(
            repository,
            instrukt_core::config::REVIEW_ROLE,
            Operation::ReviewPublish,
            clock,
        )
    }
    pub(crate) fn proposal_client(
        &self,
        repository: &str,
        clock: &dyn Fn() -> Timestamp,
    ) -> Result<super::_github::GitHub, AppError> {
        self.role_client(
            repository,
            instrukt_core::config::COMPANY_AGENT_ROLE,
            Operation::ProposalOpen,
            clock,
        )
    }
    fn role_client(
        &self,
        repository: &str,
        role: &str,
        operation: Operation,
        clock: &dyn Fn() -> Timestamp,
    ) -> Result<super::_github::GitHub, AppError> {
        let clock = Clock::starting(clock());
        self.with_apps(|apps| {
            let credential = apps.issue(repository, Some(role), operation, clock)?;
            Ok(
                super::_github::GitHub::new(&apps.api, repository, credential.token().to_owned())
                    .with_gate(apps.gate.clone()),
            )
        })
    }
}
impl instrukt_core::interfaces::GitHubActors for GitHubAccess {
    fn actor(&self, role: &str) -> Result<Bot, AppError> {
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
            let gh = super::_github::GitHub::new(&apps.api, repository, record.token().into())
                .with_gate(apps.gate.clone());
            let inspect = || -> Result<(), super::GitHubError> {
                if gh.read("")?["has_issues"] == false {
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
            let gh = super::_github::GitHub::new(&apps.api, repository, account.token().into())
                .with_gate(apps.gate.clone());
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

fn permissions_list(operation: Operation) -> &'static [(&'static str, &'static str)] {
    match operation {
        Operation::Record => &[
            ("issues", "write"),
            ("contents", "read"),
            ("pull_requests", "read"),
        ],
        Operation::Author => &[("contents", "write"), ("pull_requests", "write")],
        Operation::Review => &[("contents", "read"), ("pull_requests", "write")],
        Operation::Board => &[("organization_projects", "write")],
        Operation::Observe => &[
            ("checks", "read"),
            ("statuses", "read"),
            ("contents", "read"),
        ],
        Operation::ReviewObserve => &[("pull_requests", "read")],
        Operation::ReviewPublish => &[("pull_requests", "write"), ("statuses", "write")],
        Operation::CompanyRepository => &[("administration", "write")],
        Operation::ProposalOpen => &[("pull_requests", "write"), ("contents", "read")],
    }
}

fn permissions_map(operation: Operation) -> HashMap<String, String> {
    permissions_list(operation)
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

struct Unready {
    identity: &'static str,
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
        let clock = Clock::starting(now);
        let answer = self.with_apps(|apps| {
            let fail = |apps: &Apps, identity, role, operation, error| Unready {
                identity,
                app: Some(apps.selected(role).slug.clone()),
                operation,
                error,
            };
            let observation = match apps.issue(repository, None, Operation::ReviewObserve, clock) {
                Ok(cred) => cred,
                Err(err) => {
                    return Ok(Err(fail(
                        apps,
                        "project",
                        None,
                        Operation::ReviewObserve,
                        err,
                    )));
                }
            };
            let role = Some(instrukt_core::config::REVIEW_ROLE);
            Ok(
                match apps.issue(repository, role, Operation::ReviewPublish, clock) {
                    Ok(_) => Ok(observation),
                    Err(err) => Err(fail(apps, "reviewer", role, Operation::ReviewPublish, err)),
                },
            )
        });
        readiness_result(
            answer,
            repository,
            "automatic review",
            "review_readiness_unmet",
            Operation::ReviewObserve,
        )
    }
}

impl instrukt_core::interfaces::ProposalReadiness for GitHubAccess {
    fn proposal_readiness(
        &self,
        repository: &str,
        now: Timestamp,
    ) -> instrukt_core::interfaces::ReviewReadiness {
        let clock = Clock::starting(now);
        let role = Some(instrukt_core::config::COMPANY_AGENT_ROLE);
        let answer = self.with_apps(|apps| {
            Ok(
                match apps.issue(repository, role, Operation::ProposalOpen, clock) {
                    Ok(cred) => Ok(cred),
                    Err(error) => Err(Unready {
                        identity: "company agent",
                        app: Some(apps.selected(role).slug.clone()),
                        operation: Operation::ProposalOpen,
                        error,
                    }),
                },
            )
        });
        readiness_result(
            answer,
            repository,
            "knowledge proposals",
            "proposal_readiness_unmet",
            Operation::ProposalOpen,
        )
    }
}

fn readiness_result(
    answer: Result<Result<Arc<Credential>, Unready>, AppError>,
    repository: &str,
    purpose: &str,
    event: &'static str,
    fallback_operation: Operation,
) -> instrukt_core::interfaces::ReviewReadiness {
    use instrukt_core::interfaces::ReviewReadiness;
    let unready = match answer {
        Ok(Ok(observation)) => return ReviewReadiness::Met { observation },
        Ok(Err(unready)) => unready,
        Err(error) => Unready {
            identity: "configured",
            app: None,
            operation: fallback_operation,
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
    let mut perms: Vec<_> = permissions_map(unready.operation).into_iter().collect();
    perms.sort_by(|a, b| a.0.cmp(&b.0));
    let needed = perms
        .into_iter()
        .map(|(name, level)| format!("{name}: {level}"))
        .collect::<Vec<_>>()
        .join(", ");
    let app = unready
        .app
        .map(|slug| format!(" App `{slug}`"))
        .unwrap_or_default();
    tracing::info!(event, repository, identity = unready.identity, error = %unready.error);
    ReviewReadiness::Unmet(format!(
        "the {} identity's{app} cannot do its part for {purpose} on {repository}: {}; grant its installation on {repository} {needed}",
        unready.identity, unready.error
    ))
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
    gate_clock: Arc<Mutex<Clock>>,
    gate: instrukt_outside::Gate,
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
        let gate_clock = Arc::new(Mutex::new(Clock::starting(Timestamp::now())));
        let reader = gate_clock.clone();
        let gate = instrukt_outside::Gate::new(
            Arc::new(move || reader.lock().unwrap_or_else(PoisonError::into_inner).now()),
            Arc::new(Instant::now),
            instrukt_outside::Report::log(),
        );
        Ok(Self {
            home: home.clone(),
            api,
            identity,
            gate_clock,
            gate,
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
        *self
            .gate_clock
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = clock;
        if let Some(credential) = self
            .credentials
            .get(&key)
            .filter(|c| c.expires.as_second() > now.as_second() + 30)
        {
            return Ok(credential.clone());
        }
        let renewal = self.credentials.contains_key(&key);
        let start = Instant::now();
        let gate = self.gate.clone();
        let gate_key = instrukt_outside::Key::new("github-app", [self.api.as_str()])
            .item(app.app_id.to_string());
        let attempt = gate.attempt(
            &gate_key,
            &instrukt_outside::Settings::default(),
            || match self
                .issue_uncached(repository, &app, operation, clock)
                .map(Arc::new)
            {
                Ok(credential) => instrukt_outside::Tried::Done(credential),
                Err(error @ AppError::RateLimited { until }) => {
                    instrukt_outside::Tried::Unavailable(
                        instrukt_outside::Unavailable {
                            reason: instrukt_outside::Reason::RateLimited,
                            extent: instrukt_outside::Extent::Key,
                            named: until.map(instrukt_outside::Named::At),
                        },
                        error,
                    )
                }
                Err(error @ AppError::Unreachable) => instrukt_outside::Tried::Unavailable(
                    instrukt_outside::Unavailable {
                        reason: instrukt_outside::Reason::Unreachable,
                        extent: instrukt_outside::Extent::Scope,
                        named: None,
                    },
                    error,
                ),
                Err(error @ AppError::Refused { status, .. }) if status >= 500 => {
                    instrukt_outside::Tried::Unavailable(
                        instrukt_outside::Unavailable {
                            reason: instrukt_outside::Reason::Unavailable {
                                status: Some(status),
                            },
                            extent: instrukt_outside::Extent::Scope,
                            named: None,
                        },
                        error,
                    )
                }
                Err(error) => instrukt_outside::Tried::Defect(error),
            },
        );
        let result = match attempt {
            instrukt_outside::Attempted::Done(credential) => Ok(credential),
            instrukt_outside::Attempted::Defect(error) => Err(error),
            instrukt_outside::Attempted::Wait {
                error: Some(error), ..
            } => Err(error),
            instrukt_outside::Attempted::Wait { wait, error: None } => {
                if wait.reason() == instrukt_outside::Reason::RateLimited {
                    Err(AppError::RateLimited {
                        until: Some(wait.at()),
                    })
                } else {
                    Err(AppError::Unreachable)
                }
            }
        };
        tracing::debug!(
            event = "github_credential_acquired",
            app_id = app.app_id,
            app_slug = %app.slug,
            repository,
            operation = ?operation,
            outcome = if result.is_err() {
                "failed"
            } else if renewal {
                "renewed"
            } else {
                "issued"
            },
            duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        );
        if result.is_err() {
            self.installations
                .remove(&(repository.to_owned(), app.app_id));
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
        let pem = instrukt_core::config::resolve_secret(&self.home, &app.private_key).map_err(
            |_| {
                setup(
                    "selected App key unavailable in owner-only secrets.env; use instrukt secrets set",
                )
            },
        )?;
        let key = octocrab::EncodingKey::from_rsa_pem(pem.as_bytes())
            .map_err(|_| setup("selected App key is not usable RSA PEM"))?;
        let app_crab = build_octocrab(&self.api, |b| {
            b.app(octocrab::models::AppId(app.app_id), key)
        })?;
        if !self.verified_apps.contains(&app.app_id) {
            let identity = get_json(&app_crab, "/app", clock)?;
            if identity["id"].as_u64() != Some(app.app_id)
                || identity["slug"].as_str() != Some(&app.slug)
            {
                return Err(setup(
                    "selected key's App identity does not match configuration",
                ));
            }
            self.verified_apps.insert(app.app_id);
        }
        let organisation = op == Operation::CompanyRepository;
        let installation_endpoint = if organisation {
            format!("/orgs/{repository}/installation")
        } else {
            format!("/repos/{repository}/installation")
        };
        let installation = match self.installations.get(&(repository.into(), app.app_id)) {
            Some(installation) => installation.clone(),
            None => get_json(&app_crab, &installation_endpoint, clock)?,
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
        let required_list = permissions_list(op);
        let required_permissions = permissions_map(op);
        check_permissions(&installation["permissions"], required_list)?;
        let unscoped = op == Operation::Board || organisation;
        let mut token_request = octocrab::models::apps::CreateInstallationAccessToken {
            permissions: Some(required_permissions.clone()),
            ..Default::default()
        };
        if !unscoped {
            let (_, repo) = repository
                .split_once('/')
                .ok_or_else(|| setup("invalid repository"))?;
            token_request.repositories = Some(vec![repo.to_owned()]);
        }
        let response = post_json(
            &app_crab,
            &format!("/app/installations/{id}/access_tokens"),
            &token_request,
            clock,
        )?;
        check_permissions(&response["permissions"], required_list)?;
        let expires: Timestamp = response["expires_at"]
            .as_str()
            .and_then(|s| s.parse::<Timestamp>().ok())
            .filter(|t| t.as_second() > clock.now().as_second() + 30)
            .ok_or_else(|| setup("issued credential has no usable expiry"))?;
        let token = response["token"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| setup("issuance returned no installation credential"))?;
        let token_crab = build_bearer_octocrab(&self.api, token)?;
        if !unscoped {
            let repos = get_json(
                &token_crab,
                "/installation/repositories?per_page=100",
                clock,
            )?;
            if !repos["repositories"].as_array().is_some_and(|rs| {
                rs.iter()
                    .any(|r| r["full_name"].as_str() == Some(repository))
            }) {
                return Err(setup("issued credential cannot reach repository"));
            }
        }
        let viewer_query = json!({"query":"query { viewer { id login databaseId } }"});
        let viewer_answer =
            self.send(Method::Post, "/graphql", token, Some(&viewer_query), clock)?;
        if !(200..300).contains(&viewer_answer.status) {
            return Err(AppError::Refused {
                operation: "/graphql".into(),
                status: viewer_answer.status,
            });
        }
        if let Some(errors) = viewer_answer.body["errors"]
            .as_array()
            .filter(|e| !e.is_empty())
            && let super::GitHubError::RateLimited { until } = super::_github::graphql_failure(
                errors,
                viewer_answer.status,
                &viewer_answer.limits,
                viewer_answer.received,
                token,
            )
        {
            return Err(AppError::RateLimited { until });
        }
        let viewer = &viewer_answer.body["data"]["viewer"];
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
        let token_crab = build_bearer_octocrab(&self.api, credential.token())?;
        let apps = self.identity.apps.values().cloned().collect::<Vec<_>>();
        for app in &apps {
            if self.actors.get(&app.bot.login) == Some(&app.bot) {
                continue;
            }
            let bot = get_json(&token_crab, &format!("/users/{}", app.bot.login), clock)?;
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
    /// One App API request answered with any status but a throttling one;
    /// used by `_company_repos` and GraphQL viewer checks.
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
        let (base_uri, route) = match (endpoint, self.api.strip_suffix("/api/v3")) {
            ("/graphql", Some(host)) => (host, "/api/graphql"),
            _ => (self.api.as_str(), endpoint),
        };
        let crab = build_bearer_octocrab(base_uri, bearer)?;
        let route = route.to_owned();
        let payload = body.cloned();
        let (status, headers, received, text) = instrukt_outside::runtime::block_on(async move {
            tokio::time::timeout(REQUEST_TIMEOUT, async move {
                let response = match method {
                    Method::Post => crab._post(route, payload.as_ref()).await,
                    Method::Patch => crab._patch(route, payload.as_ref()).await,
                    Method::Put => crab._put(route, payload.as_ref()).await,
                }
                .map_err(|_| AppError::Unreachable)?;
                let received = clock.now();
                let status = response.status().as_u16();
                let headers = response.headers().clone();
                // Judge throttling from status + headers BEFORE reading the body so
                // a body timeout or decode failure on 429/403 cannot discard the retry deadline.
                if let Some(until) = super::_github::judge_rate_limit(status, &headers, received) {
                    return Err(AppError::RateLimited { until });
                }
                let text = crab
                    .body_to_string(response)
                    .await
                    .map_err(|_| AppError::Unreachable)?;
                Ok::<_, AppError>((status, headers, received, text))
            })
            .await
            .map_err(|_| AppError::Unreachable)?
        })
        .map_err(|_| AppError::Unreachable)??;
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
            limits: super::_github::Limits::read(&headers),
            received,
        })
    }
}

fn get_json(crab: &octocrab::Octocrab, endpoint: &str, clock: Clock) -> Result<Value, AppError> {
    let (crab, route) = (crab.clone(), endpoint.to_owned());
    run_octocrab(endpoint, clock, async move {
        crab.get::<Value, _, _>(route, None::<&()>).await
    })
}

fn post_json<B: serde::Serialize>(
    crab: &octocrab::Octocrab,
    endpoint: &str,
    body: &B,
    clock: Clock,
) -> Result<Value, AppError> {
    let (crab, route) = (crab.clone(), endpoint.to_owned());
    let payload = serde_json::to_value(body).map_err(|_| setup("unreadable App API request"))?;
    run_octocrab(endpoint, clock, async move {
        crab.post::<_, Value>(route, Some(&payload)).await
    })
}

fn run_octocrab<F>(endpoint: &str, clock: Clock, future: F) -> Result<Value, AppError>
where
    F: std::future::Future<Output = octocrab::Result<Value>> + Send,
{
    let outcome = instrukt_outside::runtime::block_on(async move {
        tokio::time::timeout(REQUEST_TIMEOUT, future)
            .await
            .map_err(|_| AppError::Unreachable)
    })
    .map_err(|_| AppError::Unreachable)??;
    match outcome {
        Ok(value) => Ok(value),
        Err(octocrab::Error::JWT { .. }) => Err(setup("cannot sign selected App JWT")),
        Err(octocrab::Error::GitHub { source, .. }) => {
            let status = source.status_code.as_u16();
            let empty = http::HeaderMap::new();
            let headers = source.headers.as_ref().unwrap_or(&empty);
            if let Some(until) = super::_github::judge_rate_limit(status, headers, clock.now()) {
                return Err(AppError::RateLimited { until });
            }
            Err(AppError::Refused {
                operation: endpoint.split('?').next().unwrap_or(endpoint).into(),
                status,
            })
        }
        Err(octocrab::Error::Serde { .. } | octocrab::Error::Json { .. }) => {
            Err(setup("unreadable App API response"))
        }
        Err(_) => Err(AppError::Unreachable),
    }
}

fn build_octocrab(
    base_uri: &str,
    configure: impl FnOnce(
        octocrab::OctocrabBuilder<
            octocrab::NoSvc,
            octocrab::DefaultOctocrabBuilderConfig,
            octocrab::NoAuth,
            octocrab::NotLayerReady,
        >,
    ) -> octocrab::OctocrabBuilder<
        octocrab::NoSvc,
        octocrab::DefaultOctocrabBuilderConfig,
        octocrab::NoAuth,
        octocrab::NotLayerReady,
    >,
) -> Result<octocrab::Octocrab, AppError> {
    let rt = instrukt_outside::runtime::runtime().map_err(|_| AppError::Unreachable)?;
    let _guard = rt.enter();
    let builder =
        super::_github::octocrab_builder(base_uri).map_err(|_| setup("invalid github.api_url"))?;
    configure(builder)
        .build()
        .map_err(|_| AppError::Unreachable)
}

fn build_bearer_octocrab(base_uri: &str, bearer: &str) -> Result<octocrab::Octocrab, AppError> {
    if http::HeaderValue::from_str(&format!("Bearer {bearer}")).is_err() {
        return Err(setup("authenticated App capability is unavailable"));
    }
    build_octocrab(base_uri, |b| b.personal_token(bearer.to_owned()))
}

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

/// Unauthenticated GET against the configured GitHub API origin through
/// [`octocrab::Octocrab`], used for public account lookup.
pub(super) fn public_get(api: &str, route: &str) -> Result<(u16, http::HeaderMap, String), String> {
    let rt = instrukt_outside::runtime::runtime().map_err(|err| err.to_string())?;
    let crab = {
        let _guard = rt.enter();
        super::_github::octocrab_builder(api)
            .map_err(|err| err.to_string())?
            .build()
            .map_err(|err| err.to_string())?
    };
    let route = route.to_owned();
    instrukt_outside::runtime::block_on(async move {
        tokio::time::timeout(REQUEST_TIMEOUT, async move {
            let response = crab._get(route).await.map_err(|err| err.to_string())?;
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            if status == 404
                || super::_github::Limits::read(&headers).refusal_throttles(status)
                || !(200..300).contains(&status)
            {
                return Ok((status, headers, String::new()));
            }
            let text = crab
                .body_to_string(response)
                .await
                .map_err(|err| err.to_string())?;
            Ok((status, headers, text))
        })
        .await
        .map_err(|_| "request timed out".to_owned())?
    })
    .map_err(|err| err.to_string())?
}

fn check_permissions(actual: &Value, required: &[(&str, &str)]) -> Result<(), AppError> {
    for &(name, wanted) in required {
        if actual[name] != wanted && !(wanted == "read" && actual[name] == "write") {
            return Err(setup(&format!(
                "installation credential lacks required {name} permission"
            )));
        }
    }
    Ok(())
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
